//! A generated corpus to try the memory on: a week of desk snapshots drawn from shapes, in the
//! layout `vision-live-desk` keeps approved cards in, and scanned documents from adk-vision's
//! generated week. Every picture is drawn here and every document's data is invented.

use std::path::Path;

use adk_vision::DecodedImage;
use adk_vision::memory::fixtures::{PageKind, WeekCorpus, WeekSpec};
use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use serde_json::json;

const W: u32 = 512;
const H: u32 = 384;
/// Where the wall ends and the desk begins.
const DESK_TOP: i32 = 150;

/// An RGBA canvas with the few shapes a desk needs.
struct Canvas {
    rgba: Vec<u8>,
}

impl Canvas {
    fn new() -> Self {
        let mut canvas = Self {
            rgba: vec![0; (W * H * 4) as usize],
        };
        for y in 0..H as i32 {
            for x in 0..W as i32 {
                let colour = if y < DESK_TOP {
                    let t = y as f32 / DESK_TOP as f32;
                    [
                        (226.0 - 18.0 * t) as u8,
                        (220.0 - 20.0 * t) as u8,
                        (208.0 - 20.0 * t) as u8,
                    ]
                } else {
                    // Wood: bands that drift across the desk.
                    let band = ((y + x / 23) / 9) % 3;
                    let shade = [0, 9, 4][band as usize];
                    [158 - shade, 108 - shade, 68 - shade / 2]
                };
                canvas.put(x, y, colour);
            }
        }
        canvas
    }

    fn put(&mut self, x: i32, y: i32, [r, g, b]: [u8; 3]) {
        if x < 0 || y < 0 || x >= W as i32 || y >= H as i32 {
            return;
        }
        let i = ((y as u32 * W + x as u32) * 4) as usize;
        self.rgba[i..i + 4].copy_from_slice(&[r, g, b, 255]);
    }

    fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, colour: [u8; 3]) {
        for y in y0..y1 {
            for x in x0..x1 {
                self.put(x, y, colour);
            }
        }
    }

    fn ellipse(&mut self, cx: i32, cy: i32, rx: i32, ry: i32, colour: [u8; 3]) {
        for y in cy - ry..=cy + ry {
            for x in cx - rx..=cx + rx {
                let (dx, dy) = ((x - cx) as f32 / rx as f32, (y - cy) as f32 / ry as f32);
                if dx * dx + dy * dy <= 1.0 {
                    self.put(x, y, colour);
                }
            }
        }
    }

    fn ring(&mut self, cx: i32, cy: i32, rx: i32, ry: i32, width: f32, colour: [u8; 3]) {
        for y in cy - ry..=cy + ry {
            for x in cx - rx..=cx + rx {
                let (dx, dy) = ((x - cx) as f32 / rx as f32, (y - cy) as f32 / ry as f32);
                let d = (dx * dx + dy * dy).sqrt();
                if d <= 1.0 && d >= 1.0 - width / rx as f32 {
                    self.put(x, y, colour);
                }
            }
        }
    }

    fn image(self) -> DecodedImage {
        DecodedImage::from_rgba(W, H, self.rgba).expect("a canvas of the right size")
    }
}

/// What stands on the desk in one snapshot.
#[derive(Clone, Copy)]
enum Thing {
    Mug([u8; 3]),
    Phone,
    Laptop,
    Papers,
    Book,
    Plant,
}

const RED: [u8; 3] = [196, 32, 38];
const BLUE: [u8; 3] = [36, 84, 178];

fn darker([r, g, b]: [u8; 3], by: f32) -> [u8; 3] {
    [
        (r as f32 * by) as u8,
        (g as f32 * by) as u8,
        (b as f32 * by) as u8,
    ]
}

fn draw(canvas: &mut Canvas, thing: Thing, x: i32) {
    match thing {
        Thing::Mug(colour) => {
            let (top, bottom) = (205, 290);
            canvas.ellipse(x + 34, bottom + 4, 46, 9, [118, 80, 50]);
            canvas.ring(x + 70, 245, 20, 26, 8.0, darker(colour, 0.85));
            canvas.rect(x, top, x + 68, bottom, colour);
            canvas.rect(
                x + 6,
                top + 6,
                x + 16,
                bottom - 6,
                [
                    colour[0].saturating_add(40),
                    colour[1].saturating_add(40),
                    colour[2].saturating_add(40),
                ],
            );
            canvas.ellipse(x + 34, bottom, 34, 8, darker(colour, 0.8));
            canvas.ellipse(x + 34, top, 34, 9, darker(colour, 0.7));
            canvas.ellipse(x + 34, top + 1, 29, 6, [70, 42, 24]);
        }
        Thing::Phone => {
            canvas.rect(x - 2, 232, x + 60, 340, [24, 24, 26]);
            canvas.rect(x + 4, 240, x + 54, 330, [44, 58, 78]);
            canvas.ellipse(x + 29, 335, 4, 2, [70, 70, 74]);
        }
        Thing::Laptop => {
            canvas.rect(x, 120, x + 210, 250, [196, 198, 204]);
            canvas.rect(x + 8, 128, x + 202, 242, [34, 38, 48]);
            canvas.rect(x + 20, 140, x + 120, 150, [86, 140, 200]);
            canvas.rect(x + 20, 160, x + 170, 166, [120, 124, 136]);
            canvas.rect(x - 16, 250, x + 226, 278, [176, 178, 186]);
            canvas.rect(x + 70, 262, x + 140, 274, [150, 152, 160]);
        }
        Thing::Papers => {
            canvas.rect(x, 240, x + 120, 352, [244, 242, 236]);
            for line in 0..8 {
                canvas.rect(
                    x + 12,
                    256 + line * 11,
                    x + 104 - (line % 3) * 14,
                    259 + line * 11,
                    [150, 150, 150],
                );
            }
        }
        Thing::Book => {
            canvas.rect(x, 250, x + 132, 300, [34, 120, 76]);
            canvas.rect(x + 4, 300, x + 128, 310, [236, 230, 214]);
            canvas.rect(x + 20, 266, x + 112, 272, [214, 186, 92]);
        }
        Thing::Plant => {
            canvas.rect(x + 8, 250, x + 64, 310, [184, 92, 54]);
            for (dx, dy, r) in [(36, 222, 30), (14, 232, 20), (58, 230, 22), (36, 196, 22)] {
                canvas.ellipse(x + dx, dy, r, r, [52, 132, 60]);
            }
        }
    }
}

fn scene(things: &[(Thing, i32)]) -> DecodedImage {
    let mut canvas = Canvas::new();
    for (thing, x) in things {
        draw(&mut canvas, *thing, *x);
    }
    canvas.image()
}

fn local(day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    let date = NaiveDate::from_ymd_opt(2026, 11, day).expect("a November date");
    let naive = date.and_hms_opt(hour, minute, 0).expect("a time");
    Local
        .from_local_datetime(&naive)
        .single()
        .expect("an unambiguous local time")
        .to_utc()
}

fn write(
    dir: &Path,
    name: &str,
    image: &DecodedImage,
    sidecar: serde_json::Value,
) -> anyhow::Result<()> {
    std::fs::write(dir.join(format!("{name}.png")), image.encode_png()?)?;
    std::fs::write(
        dir.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&sidecar)?,
    )?;
    Ok(())
}

/// Writes `snapshots/`, `documents/` and `queries/` under `out`, and returns what it wrote.
pub fn generate(out: &Path) -> anyhow::Result<String> {
    use Thing::{Book, Laptop, Mug, Papers, Phone, Plant};
    let snapshots = out.join("snapshots");
    let documents = out.join("documents");
    let queries = out.join("queries");
    for dir in [&snapshots, &documents, &queries] {
        std::fs::create_dir_all(dir)?;
    }

    // A week on the desk. The red mug is last seen on Friday afternoon.
    let week: [(u32, u32, u32, Vec<(Thing, i32)>); 12] = [
        (16, 9, 10, vec![(Laptop, 60), (Mug(RED), 360)]),
        (16, 14, 30, vec![(Laptop, 60), (Papers, 340)]),
        (17, 10, 5, vec![(Mug(RED), 120), (Phone, 330)]),
        (17, 16, 20, vec![(Mug(BLUE), 90), (Book, 280)]),
        (18, 11, 0, vec![(Laptop, 150)]),
        (18, 15, 45, vec![(Papers, 80), (Mug(RED), 330)]),
        (19, 9, 30, vec![(Plant, 100), (Phone, 320)]),
        (19, 17, 10, vec![(Laptop, 40), (Mug(BLUE), 380)]),
        (20, 10, 40, vec![(Book, 70), (Papers, 300)]),
        (
            20,
            16,
            40,
            vec![(Laptop, 30), (Mug(RED), 300), (Phone, 420)],
        ),
        (21, 11, 15, vec![(Mug(BLUE), 160), (Plant, 330)]),
        (21, 18, 0, vec![(Laptop, 60), (Book, 330)]),
    ];
    for (n, (day, hour, minute, things)) in week.iter().enumerate() {
        let at = local(*day, *hour, *minute);
        let name = format!("desk-{}", at.with_timezone(&Local).format("%Y%m%d-%H%M%S"));
        write(
            &snapshots,
            &name,
            &scene(things),
            json!({
                "caption": format!("Desk camera snapshot {}", n + 1),
                "source": "desk-camera",
                "capturedAt": at.to_rfc3339(),
            }),
        )?;
    }

    // Scanned paperwork from adk-vision's generated week: delivery notes, inspection reports
    // and invoices, three of each.
    let corpus = WeekCorpus::new(WeekSpec::default());
    let mut counts = std::collections::BTreeMap::new();
    let mut pages = 0;
    for item in corpus.items() {
        let Some(kind) = item.page else { continue };
        let taken = counts.entry(format!("{kind:?}")).or_insert(0);
        if *taken == 3 {
            continue;
        }
        *taken += 1;
        pages += 1;
        write(
            &documents,
            &format!("scan-{pages:02}"),
            &corpus.render(item),
            json!({
                "caption": format!("Scanned page {pages}"),
                "source": "documents",
                "capturedAt": item.captured_at.to_rfc3339(),
            }),
        )?;
    }

    // Query images that are in no folder: an invoice from another week, and a red mug in a
    // scene the memory has not seen.
    let other = WeekCorpus::new(WeekSpec {
        seed: 0x0DD5_EED5,
        ..WeekSpec::default()
    });
    let invoice = other
        .items()
        .iter()
        .find(|item| item.page == Some(PageKind::Invoice))
        .ok_or_else(|| anyhow::anyhow!("the generated week has no invoice"))?;
    std::fs::write(
        queries.join("receipt-example.png"),
        other.render(invoice).encode_png()?,
    )?;
    std::fs::write(
        queries.join("mug-example.png"),
        scene(&[(Mug(RED), 220)]).encode_png()?,
    )?;

    Ok(format!(
        "wrote {} desk snapshots to {}, {pages} scanned pages to {}, and 2 query images to {}",
        week.len(),
        snapshots.display(),
        documents.display(),
        queries.display()
    ))
}
