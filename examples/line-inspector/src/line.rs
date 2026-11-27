//! The line: trays fed through adk-vision's stream pipeline like frames from a camera over
//! the conveyor, every candle inspected on this computer, passed, rejected with its defect
//! segmented and measured, or handed to the agent when it is borderline.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adk_vision::stream::{
    FrameSource, FrameStream, Observation, ObservationSink, StreamPipeline, StreamSummary,
};
use adk_vision::{
    BBox, Calibration, DetectQuery, FiducialSpec, Frame, FramePixels, FrameSourceId, Mask,
    Sampling, SegmentPrompt, Shape, SourceImage, VisionCx, VisionRuntime,
};
use async_trait::async_trait;
use futures::StreamExt;
use serde::Serialize;
use tokio::sync::mpsc;

use crate::data::Tray;
use crate::inspect::{CANDLE_MM, SHOWN_PASS, SHOWN_REJECT, load};

/// The camera over the conveyor: one tray a second, paced like the line or as fast as the
/// inspection keeps up.
pub struct TrayFeed {
    id: FrameSourceId,
    trays: Vec<Tray>,
    paced: bool,
}

impl TrayFeed {
    pub fn new(trays: Vec<Tray>, paced: bool) -> Self {
        Self {
            id: FrameSourceId("line-2".into()),
            trays,
            paced,
        }
    }
}

#[async_trait]
impl FrameSource for TrayFeed {
    fn id(&self) -> &FrameSourceId {
        &self.id
    }

    async fn frames(&self, _sampling: Sampling) -> adk_vision::Result<FrameStream> {
        let (id, trays, paced) = (self.id.clone(), self.trays.clone(), self.paced);
        Ok(futures::stream::unfold(0usize, move |i| {
            let (id, trays) = (id.clone(), trays.clone());
            async move {
                let tray = trays.get(i)?;
                if paced {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                let frame = load(&tray.image)
                    .map(|pixels| Frame {
                        source: id,
                        index: i as u64,
                        pts: Duration::from_secs(i as u64),
                        captured_at: chrono::Utc::now(),
                        pixels: FramePixels::Ephemeral(Arc::new(pixels)),
                    })
                    .map_err(|e| adk_vision::VisionError::InvalidInput(e.to_string()));
                Some((frame, i + 1))
            }
        })
        .boxed())
    }
}

/// What the line did with one candle.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    pub id: String,
    pub tray: String,
    pub candle: [f32; 4],
    pub suspect: [f32; 4],
    /// On the board's scale: pass below 0.45, reject above 0.75.
    pub score: f32,
    /// `pass`, `reject` or `borderline`, as the inspection on this computer called it.
    pub stage: String,
    /// The defect's size, for a part the line rejected.
    pub longest_mm: Option<f64>,
    pub area_mm2: Option<f64>,
    /// The segmented region's overlap with the dataset's mask, for a defective part.
    pub iou: Option<f32>,
    /// Whether the dataset marks a defect on this candle, and VisA's name for it.
    pub defective: bool,
    pub defect: Option<String>,
}

/// Every part the line has seen, the reject log and who to tell about a borderline one.
pub struct Book {
    pub parts: Mutex<BTreeMap<String, Part>>,
    pub log: Mutex<Vec<String>>,
    pub trays_seen: Mutex<Vec<String>>,
    /// Each reject's segmented defect, in the tray's pixels, for the part's crop on the page.
    pub regions: Mutex<BTreeMap<String, Mask>>,
    borderline: mpsc::UnboundedSender<(Part, Vec<u8>)>,
}

impl Book {
    pub fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<(Part, Vec<u8>)>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            Arc::new(Self {
                parts: Mutex::new(BTreeMap::new()),
                log: Mutex::new(Vec::new()),
                trays_seen: Mutex::new(Vec::new()),
                regions: Mutex::new(BTreeMap::new()),
                borderline: tx,
            }),
            rx,
        )
    }

    pub fn get(&self, id: &str) -> Option<Part> {
        self.parts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    pub fn all(&self) -> Vec<Part> {
        self.parts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect()
    }

    pub fn note(&self, line: String) {
        println!("{line}");
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(line);
    }
}

/// The line's sink: each tray's candles decided as they come off the inspection.
pub struct Line {
    pub trays: Vec<Tray>,
    pub book: Arc<Book>,
    pub segmenter: Arc<VisionRuntime>,
}

fn arr(b: &BBox) -> [f32; 4] {
    [b.x0.round(), b.y0.round(), b.x1.round(), b.y1.round()]
}

/// The dataset's mask over `candle`: whether it marks a defect there.
fn truth(mask: &Option<image::GrayImage>, candle: &BBox) -> bool {
    let Some(mask) = mask else { return false };
    let (w, h) = mask.dimensions();
    (candle.y0.max(0.0) as u32..(candle.y1 as u32).min(h)).any(|y| {
        (candle.x0.max(0.0) as u32..(candle.x1 as u32).min(w))
            .any(|x| mask.get_pixel(x, y).0[0] > 0)
    })
}

/// The overlap of a segmented region and the dataset's mask, inside the candle's box.
fn iou(found: &Mask, truth: &image::GrayImage, candle: &BBox) -> f32 {
    let bits = found.decode();
    let (w, h) = (found.width, found.height);
    let (mut both, mut either) = (0u64, 0u64);
    for y in candle.y0.max(0.0) as u32..(candle.y1 as u32).min(h) {
        for x in candle.x0.max(0.0) as u32..(candle.x1 as u32).min(w) {
            let a = bits[(y * w + x) as usize];
            let b = truth.get_pixel(x, y).0[0] > 0;
            both += u64::from(a && b);
            either += u64::from(a || b);
        }
    }
    if either == 0 {
        0.0
    } else {
        both as f32 / either as f32
    }
}

impl Line {
    /// SAM 2.1 on this computer segments the defect from its tile; the candle's 38 mm
    /// diameter calibrates the tray's plane.
    async fn measure(
        &self,
        tray: &SourceImage,
        candle: &BBox,
        suspect: &BBox,
    ) -> anyhow::Result<(Mask, f64, f64)> {
        let found =
            crate::inspect::defect_region(tray.pixels(), candle, suspect).ok_or_else(|| {
                anyhow::anyhow!("nothing in the suspect area stands out from the wax")
            })?;
        let found_box = found
            .bbox()
            .ok_or_else(|| anyhow::anyhow!("an empty region"))?;
        // SAM 2.1 refines the region from its box; when it takes in much more than the
        // region, it has found the candle, and the region stands.
        let prompt = SegmentPrompt::Box {
            bbox: found_box.expanded(8.0),
        };
        let refined = self
            .segmenter
            .segment(&VisionCx::new(), tray, &[prompt])
            .await
            .ok()
            .and_then(|out| {
                out.value.into_iter().find_map(|region| match region.shape {
                    Shape::Mask(mask) if mask.area() <= found.area().max(1) * 4 => Some(mask),
                    _ => None,
                })
            });
        let mask = refined.unwrap_or(found);
        let size = tray.size();
        let ruler = Shape::Mask(Mask::from_box(size.0, size.1, candle));
        let calibration =
            Calibration::from_fiducial(&ruler, FiducialSpec::square(CANDLE_MM), size)?;
        let measured = calibration.measure(&Shape::Mask(mask.clone()), size)?;
        Ok((
            mask,
            (measured.longest_mm * 10.0).round() / 10.0,
            (measured.area_mm2 * 10.0).round() / 10.0,
        ))
    }
}

#[async_trait]
impl ObservationSink for Line {
    async fn observe(&self, _cx: &VisionCx, observation: Observation) -> adk_vision::Result<()> {
        let Some(tray) = self.trays.get(observation.frame.index as usize) else {
            return Ok(());
        };
        self.book
            .trays_seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(tray.id.clone());
        let gt = tray
            .mask
            .as_ref()
            .and_then(|p| image::open(p).ok())
            .map(|m| m.to_luma8());
        let pixels = observation.image.pixels().clone();
        let source = SourceImage::from_pixels(pixels)?;
        let regions = observation.regions;
        for (k, pair) in regions.chunks(2).enumerate() {
            let [candle, suspect] = pair else { continue };
            let (Some(cbox), Some(sbox)) = (candle.shape.bbox(), suspect.shape.bbox()) else {
                continue;
            };
            let score = candle.score.unwrap_or(0.0);
            let defective = truth(&gt, &cbox);
            let stage = if score < SHOWN_PASS {
                "pass"
            } else if score >= SHOWN_REJECT {
                "reject"
            } else {
                "borderline"
            };
            let mut part = Part {
                id: format!("{}-p{}", tray.id, k + 1),
                tray: tray.id.clone(),
                candle: arr(&cbox),
                suspect: arr(&sbox),
                score: (score * 100.0).round() / 100.0,
                stage: stage.into(),
                longest_mm: None,
                area_mm2: None,
                iou: None,
                defective,
                defect: defective.then(|| tray.defect.clone()).flatten(),
            };
            if stage == "reject" {
                match self.measure(&source, &cbox, &sbox).await {
                    Ok((mask, longest, area)) => {
                        part.longest_mm = Some(longest);
                        part.area_mm2 = Some(area);
                        part.iou = gt
                            .as_ref()
                            .filter(|_| defective)
                            .map(|g| (iou(&mask, g, &cbox) * 100.0).round() / 100.0);
                        self.book
                            .regions
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(part.id.clone(), mask);
                    }
                    Err(err) => {
                        tracing::warn!(part = %part.id, error = %err, "measuring the reject failed")
                    }
                }
                self.book.note(format!(
                    "[reject] {} score {:.2}: {} mm across, {} mm²",
                    part.id,
                    part.score,
                    part.longest_mm.map_or("?".into(), |v| format!("{v:.1}")),
                    part.area_mm2.map_or("?".into(), |v| format!("{v:.0}"))
                ));
            } else if stage == "borderline" {
                self.book
                    .note(format!("[second look] {} score {:.2}", part.id, part.score));
                let bytes = std::fs::read(&tray.image).unwrap_or_default();
                let _ = self.book.borderline.send((part.clone(), bytes));
            }
            self.book
                .parts
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(part.id.clone(), part);
        }
        Ok(())
    }
}

/// Runs the line: the feed through the pipeline with the inspector as its detector.
pub async fn run(
    feed: TrayFeed,
    inspector: Arc<VisionRuntime>,
    line: Arc<Line>,
) -> anyhow::Result<StreamSummary> {
    let pipeline = StreamPipeline::builder(Arc::new(feed))
        .sampling(Sampling::at(1.0))
        .detector(
            inspector,
            DetectQuery::Classes(vec!["candle".into(), "suspect".into()]),
        )
        .sink(line)
        .build()?;
    Ok(pipeline.run(&VisionCx::new()).await?)
}
