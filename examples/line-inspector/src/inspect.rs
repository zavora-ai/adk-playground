//! The inspection on this computer: each candle of a tray found, cut into nine overlapping
//! tiles, every tile embedded with SigLIP 2 and compared with the same tile of known-good
//! candles. A candle scores as its least familiar tile; that tile is where to look.

use std::path::Path;
use std::sync::Arc;

use adk_vision::{
    BBox, BackendDescriptor, CallOptions, CoordFrame, DecodedImage, DetectQuery, Detector,
    Locality, RawRegion, SentImage, SourceImage, VisionCx, VisionRuntime,
};
use async_trait::async_trait;
use image::{DynamicImage, RgbaImage, imageops::FilterType};
use serde::{Deserialize, Serialize};

/// A candle crop is scaled to this square before it is cut into tiles.
const CROP: u32 = 384;
/// The score the board shows at the pass and the reject threshold.
pub const SHOWN_PASS: f32 = 0.45;
pub const SHOWN_REJECT: f32 = 0.75;
/// A tea light is 38 mm across: the candle is its own ruler.
pub const CANDLE_MM: f64 = 38.0;

/// Each candle's square box in the tray, found by the bright wax on the dark mat, one per
/// quarter of the tray.
pub fn locate(tray: &DecodedImage) -> Vec<BBox> {
    let (w, h) = tray.size();
    let rgba = tray.rgba();
    let bright = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        u32::from(rgba[i]) + u32::from(rgba[i + 1]) + u32::from(rgba[i + 2]) > 330
    };
    let mut boxes = Vec::new();
    for (qy, qx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
        let (x0, x1) = (qx * w / 2, (qx + 1) * w / 2);
        let (y0, y1) = (qy * h / 2, (qy + 1) * h / 2);
        let (mut xs, mut ys) = (Vec::new(), Vec::new());
        for y in (y0..y1).step_by(2) {
            for x in (x0..x1).step_by(2) {
                if bright(x, y) {
                    xs.push(x);
                    ys.push(y);
                }
            }
        }
        if xs.len() < 250 {
            continue;
        }
        xs.sort_unstable();
        ys.sort_unstable();
        let pct = |v: &[u32], p: f64| v[((v.len() - 1) as f64 * p) as usize] as f32;
        let (left, right) = (pct(&xs, 0.01), pct(&xs, 0.99));
        let (top, bottom) = (pct(&ys, 0.01), pct(&ys, 0.99));
        let (cx, cy) = ((left + right) / 2.0, (top + bottom) / 2.0);
        let r = (right - left).max(bottom - top) / 2.0 + 20.0;
        boxes.push(BBox::new(cx - r, cy - r, cx + r, cy + r));
    }
    boxes
}

fn to_image(pixels: &DecodedImage) -> RgbaImage {
    let (w, h) = pixels.size();
    RgbaImage::from_raw(w, h, pixels.rgba().to_vec()).expect("an RGBA buffer of its own size")
}

fn from_image(image: RgbaImage) -> anyhow::Result<SourceImage> {
    let (w, h) = image.dimensions();
    Ok(SourceImage::from_pixels(DecodedImage::from_rgba(
        w,
        h,
        image.into_raw(),
    )?)?)
}

/// The nine tiles of a candle, and each tile's box in the tray's pixels.
pub fn tiles(tray: &DecodedImage, candle: &BBox) -> anyhow::Result<Vec<(SourceImage, BBox)>> {
    let full = to_image(tray);
    let (w, h) = full.dimensions();
    let (x0, y0) = (candle.x0.max(0.0) as u32, candle.y0.max(0.0) as u32);
    let (x1, y1) = ((candle.x1 as u32).min(w), (candle.y1 as u32).min(h));
    let crop = DynamicImage::ImageRgba8(full)
        .crop_imm(x0, y0, x1 - x0, y1 - y0)
        .resize_exact(CROP, CROP, FilterType::Triangle);
    let scale = (x1 - x0) as f32 / CROP as f32;
    let mut out = Vec::with_capacity(9);
    for ty in 0..3 {
        for tx in 0..3 {
            let left = if tx == 0 { 0 } else { tx * 128 - 32 };
            let top = if ty == 0 { 0 } else { ty * 128 - 32 };
            let right = (tx * 128 + 160).min(CROP);
            let bottom = (ty * 128 + 160).min(CROP);
            let tile = crop
                .crop_imm(left, top, right - left, bottom - top)
                .to_rgba8();
            let bbox = BBox::new(
                x0 as f32 + left as f32 * scale,
                y0 as f32 + top as f32 * scale,
                x0 as f32 + right as f32 * scale,
                y0 as f32 + bottom as f32 * scale,
            );
            out.push((from_image(tile)?, bbox));
        }
    }
    Ok(out)
}

/// The known-good tiles, one set per tile position, unit length.
#[derive(Default, Serialize, Deserialize)]
pub struct Bank {
    pub tiles: Vec<Vec<Vec<f32>>>,
}

/// The scores a calibration of good trays sets.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thresholds {
    /// Below this a candle passes: the 97th percentile of good candles, so about one good
    /// candle in thirty gets a second look.
    pub pass: f32,
    /// Above this it is rejected: the highest good candle.
    pub reject: f32,
}

impl Thresholds {
    /// From the calibration's good candles, sorted: pass at their 97th percentile, reject
    /// above the highest.
    pub fn from_good(good: &[f32]) -> Self {
        let pass = good[((good.len() - 1) as f64 * 0.97) as usize];
        Self {
            pass,
            reject: *good.last().unwrap_or(&1.0),
        }
    }

    /// The raw score on the board's scale: 0.45 at the pass threshold, 0.75 at the reject.
    pub fn shown(&self, raw: f32) -> f32 {
        let slope = (SHOWN_REJECT - SHOWN_PASS) / (self.reject - self.pass).max(1e-6);
        (SHOWN_PASS + (raw - self.pass) * slope).clamp(0.0, 1.0)
    }
}

fn unit(v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    v.into_iter().map(|x| x / n).collect()
}

/// SigLIP 2 on this computer, the bank and the thresholds.
pub struct Scorer {
    pub embedder: Arc<VisionRuntime>,
    pub bank: Bank,
    pub thresholds: Thresholds,
}

/// One candle's inspection.
#[derive(Clone, Debug)]
pub struct Scored {
    pub candle: BBox,
    pub raw: f32,
    pub suspect: BBox,
}

impl Scorer {
    async fn embed(
        embedder: &VisionRuntime,
        images: &[SourceImage],
    ) -> anyhow::Result<Vec<Vec<f32>>> {
        let refs: Vec<&SourceImage> = images.iter().collect();
        let out = embedder
            .embed_images(&VisionCx::new(), &refs, &CallOptions::default())
            .await?;
        Ok(out.value.into_iter().map(unit).collect())
    }

    /// The bank and the thresholds from the bank and calibration trays, cached under `dir`.
    pub async fn open(
        embedder: Arc<VisionRuntime>,
        dir: &Path,
        bank_trays: &[std::path::PathBuf],
        calibration_trays: &[std::path::PathBuf],
    ) -> anyhow::Result<Self> {
        let cache = dir.join("bank.json");
        if let Ok(bytes) = std::fs::read(&cache)
            && let Ok((bank, good)) = serde_json::from_slice::<(Bank, Vec<f32>)>(&bytes)
        {
            return Ok(Self {
                embedder,
                bank,
                thresholds: Thresholds::from_good(&good),
            });
        }
        println!(
            "  building the bank from {} good trays (once; cached in data/bank.json)…",
            bank_trays.len()
        );
        let mut bank = Bank {
            tiles: vec![Vec::new(); 9],
        };
        for path in bank_trays {
            let tray = load(path)?;
            for candle in locate(&tray) {
                let tiles: Vec<SourceImage> =
                    tiles(&tray, &candle)?.into_iter().map(|(t, _)| t).collect();
                for (k, v) in Self::embed(&embedder, &tiles)
                    .await?
                    .into_iter()
                    .enumerate()
                {
                    bank.tiles[k].push(v);
                }
            }
        }
        let mut scorer = Self {
            embedder,
            bank,
            thresholds: Thresholds {
                pass: 0.0,
                reject: 1.0,
            },
        };
        println!(
            "  calibrating on {} more good trays…",
            calibration_trays.len()
        );
        let mut good = Vec::new();
        for path in calibration_trays {
            let tray = load(path)?;
            for s in scorer.score(&tray).await? {
                good.push(s.raw);
            }
        }
        good.sort_by(f32::total_cmp);
        scorer.thresholds = Thresholds::from_good(&good);
        std::fs::write(&cache, serde_json::to_vec(&(&scorer.bank, &good))?)?;
        Ok(scorer)
    }

    /// Every candle of a tray with its score and least familiar tile.
    pub async fn score(&self, tray: &DecodedImage) -> anyhow::Result<Vec<Scored>> {
        let mut out = Vec::new();
        for candle in locate(tray) {
            let tiles = tiles(tray, &candle)?;
            let images: Vec<SourceImage> = tiles.iter().map(|(t, _)| t.clone()).collect();
            let vectors = Self::embed(&self.embedder, &images).await?;
            let mut worst = (0.0f32, 0usize);
            for (k, v) in vectors.iter().enumerate() {
                let nearest = self.bank.tiles[k]
                    .iter()
                    .map(|b| b.iter().zip(v).map(|(x, y)| x * y).sum::<f32>())
                    .fold(f32::MIN, f32::max);
                let distance = 1.0 - nearest;
                if distance > worst.0 {
                    worst = (distance, k);
                }
            }
            out.push(Scored {
                candle,
                raw: worst.0,
                suspect: tiles[worst.1].1,
            });
        }
        Ok(out)
    }
}

/// The defect inside the suspect tile, found on this computer: the pixels whose colour is far
/// from the candle's own wax, grown from the farthest one. Answers its mask over the tray, or
/// `None` when nothing in the tile stands out.
pub fn defect_region(
    tray: &DecodedImage,
    candle: &BBox,
    suspect: &BBox,
) -> Option<adk_vision::Mask> {
    let (w, h) = tray.size();
    let rgba = tray.rgba();
    let px = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        [
            f32::from(rgba[i]),
            f32::from(rgba[i + 1]),
            f32::from(rgba[i + 2]),
        ]
    };
    // The wax colour: the median of the candle's inner disc.
    let (cx, cy) = ((candle.x0 + candle.x1) / 2.0, (candle.y0 + candle.y1) / 2.0);
    let r = (candle.x1 - candle.x0) / 2.0 - 20.0;
    let mut wax: Vec<[f32; 3]> = Vec::new();
    for y in ((cy - r * 0.7) as u32..(cy + r * 0.7) as u32).step_by(4) {
        for x in ((cx - r * 0.7) as u32..(cx + r * 0.7) as u32).step_by(4) {
            if x < w && y < h {
                wax.push(px(x, y));
            }
        }
    }
    if wax.is_empty() {
        return None;
    }
    let median = |c: usize| {
        let mut v: Vec<f32> = wax.iter().map(|p| p[c]).collect();
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    };
    let base = [median(0), median(1), median(2)];
    let far = |p: [f32; 3]| {
        ((p[0] - base[0]).powi(2) + (p[1] - base[1]).powi(2) + (p[2] - base[2]).powi(2)).sqrt()
    };
    let inside = |x: u32, y: u32| {
        let (dx, dy) = (x as f32 - cx, y as f32 - cy);
        (dx * dx + dy * dy).sqrt() <= r
    };
    let (x0, y0) = (suspect.x0.max(0.0) as u32, suspect.y0.max(0.0) as u32);
    let (x1, y1) = ((suspect.x1 as u32).min(w), (suspect.y1 as u32).min(h));
    let mut seed = None;
    let mut best = 0.0f32;
    for y in y0..y1 {
        for x in x0..x1 {
            let d = far(px(x, y));
            if inside(x, y) && d > best {
                best = d;
                seed = Some((x, y));
            }
        }
    }
    let (sx, sy) = seed.filter(|_| best > 60.0)?;
    let limit = best * 0.5;
    let mut bits = vec![false; (w * h) as usize];
    let mut stack = vec![(sx, sy)];
    let mut area = 0u32;
    while let Some((x, y)) = stack.pop() {
        let i = (y * w + x) as usize;
        if bits[i]
            || x < x0
            || y < y0
            || x >= x1
            || y >= y1
            || !inside(x, y)
            || far(px(x, y)) < limit
        {
            continue;
        }
        bits[i] = true;
        area += 1;
        if area > 40_000 {
            break;
        }
        stack.extend([
            (x + 1, y),
            (x.saturating_sub(1), y),
            (x, y + 1),
            (x, y.saturating_sub(1)),
        ]);
    }
    Some(adk_vision::Mask::encode(w, h, &bits))
}

pub fn load(path: &Path) -> anyhow::Result<DecodedImage> {
    let image = image::open(path)?.to_rgba8();
    let (w, h) = image.dimensions();
    Ok(DecodedImage::from_rgba(w, h, image.into_raw())?)
}

/// The inspection as an adk-vision detector, so the line's stream pipeline runs it like any
/// other: each candle a `candle` region scored on the board's scale, its least familiar tile a
/// `suspect` region with the same score.
pub struct Inspector {
    descriptor: BackendDescriptor,
    scorer: Arc<Scorer>,
}

impl Inspector {
    pub fn new(scorer: Arc<Scorer>) -> Self {
        Self {
            descriptor: BackendDescriptor::new(
                "candle-inspector",
                Locality::InProcess,
                CoordFrame::SentPixels,
            ),
            scorer,
        }
    }
}

#[async_trait]
impl Detector for Inspector {
    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    async fn detect(
        &self,
        _cx: &VisionCx,
        image: &SentImage,
        _query: &DetectQuery,
    ) -> adk_vision::Result<Vec<RawRegion>> {
        let scored = self.scorer.score(image.pixels()).await.map_err(|e| {
            adk_vision::VisionError::InvalidInput(format!("inspecting the tray failed: {e}"))
        })?;
        let t = self.scorer.thresholds;
        Ok(scored
            .into_iter()
            .flat_map(|s| {
                let shown = t.shown(s.raw);
                [
                    RawRegion::boxed(s.candle, "candle", shown),
                    RawRegion::boxed(s.suspect, "suspect", shown),
                ]
            })
            .collect())
    }
}
