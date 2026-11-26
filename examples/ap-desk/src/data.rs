//! The dataset and the books: FATURA invoices (CC BY 4.0) fetched by checksum, an inbox of
//! forty-four documents built from them, the ERP the desk matches against, and what each
//! document should come to.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use futures::StreamExt;
use image::{DynamicImage, ImageBuffer, Rgb, RgbImage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// FATURA on Zenodo, record 8261508: 10,000 synthetic invoices in 50 layouts.
pub const ZIP_URL: &str = "https://zenodo.org/records/8261508/files/FATURA.zip?download=1";
pub const ZIP_SIZE: u64 = 363_504_027;
pub const ZIP_SHA256: &str = "c8bf2ee6ba1b7a678954b7b89b31f9387100dc6b9e990c076aff5a232a9f53d7";

/// The ten suppliers of the batch, one FATURA template each; the first four print a PO number.
pub const TEMPLATES: [u32; 10] = [3, 5, 6, 47, 2, 7, 8, 9, 10, 12];
/// Invoices per supplier, taken from FATURA's test split in instance order.
const PER_TEMPLATE: usize = 4;
/// The supplier the ERP does not know: the desk's "unknown supplier" case.
const UNAPPROVED_TEMPLATE: u32 = 12;
/// Invoices without a PO post up to this total, in the invoice's currency.
pub const NON_PO_LIMIT: f64 = 1_000.0;
/// An invoice matches its PO within this share of the PO amount.
pub const PO_TOLERANCE: f64 = 0.02;

pub fn data_dir(root: &Path) -> PathBuf {
    root.join("data")
}

pub fn inbox_dir(root: &Path) -> PathBuf {
    data_dir(root).join("inbox")
}

/// How a document reached the desk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arrival {
    /// The PDF's own rendering.
    Clean,
    /// A phone photo of the paper: tilted, soft, noisy and compressed.
    Phone,
    /// A second copy the supplier sent again, scanned.
    Resent,
}

/// What a document should come to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Post,
    Duplicate,
    OverPo,
    NotReceived,
    UnknownSupplier,
    OverLimit,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Post => "post",
            Self::Duplicate => "duplicate",
            Self::OverPo => "over_po",
            Self::NotReceived => "not_received",
            Self::UnknownSupplier => "unknown_supplier",
            Self::OverLimit => "over_limit",
        }
    }
}

/// An invoice's header as the annotations give it, normalised.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    pub supplier: String,
    pub number: String,
    pub date: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub po: Option<String>,
    pub total: f64,
    pub currency: String,
}

/// One document in the inbox and what it should come to.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Expected {
    pub doc: String,
    pub source: String,
    pub template: u32,
    pub arrival: Arrival,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<String>,
    pub header: Header,
    pub decision: Decision,
}

/// The ERP the desk matches against.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Erp {
    /// Approved suppliers by name.
    pub vendors: BTreeMap<String, Vendor>,
    /// Purchase orders by [`po_key`]: FATURA's two-digit PO numbers repeat across suppliers.
    pub purchase_orders: BTreeMap<String, PurchaseOrder>,
    /// Invoices already in the books, by supplier.
    #[serde(default)]
    pub posted: BTreeMap<String, Vec<Posted>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vendor {
    pub id: String,
    pub currency: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseOrder {
    pub number: String,
    pub supplier: String,
    pub amount: f64,
    pub currency: String,
    /// The goods receipt, when the goods have arrived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goods_receipt: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Posted {
    pub doc: String,
    pub number: String,
    pub date: String,
    pub total: f64,
    pub currency: String,
}

/// A purchase order's key: its supplier and its number, leading zeros dropped.
pub fn po_key(supplier: &str, po: &str) -> String {
    let supplier: String = supplier
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase();
    let number = po
        .trim()
        .trim_start_matches(['#', ':', ' '])
        .trim_start_matches('0');
    format!("{supplier}|{number}")
}

/// Everything `fetch` writes besides the inbox images.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Books {
    pub expected: Vec<Expected>,
    pub erp: Erp,
}

impl Books {
    pub fn load(root: &Path) -> anyhow::Result<Self> {
        let path = data_dir(root).join("books.json");
        let bytes = std::fs::read(&path).with_context(|| {
            format!(
                "{} is missing; run `cargo run --release -- fetch` first",
                path.display()
            )
        })?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

// ---------------------------------------------------------------------------------------------
// Fetch
// ---------------------------------------------------------------------------------------------

/// Downloads FATURA (checked by size and SHA-256), takes the batch's invoices and their
/// annotations out of the archive, and writes the inbox, the ERP and the expected answers.
pub async fn fetch(root: &Path) -> anyhow::Result<()> {
    let dir = data_dir(root);
    std::fs::create_dir_all(&dir)?;
    let zip_path = dir.join("FATURA.zip");
    if !(zip_path.exists() && sha256_file(&zip_path)? == ZIP_SHA256) {
        println!("  fetching FATURA.zip ({ZIP_SIZE} bytes) from Zenodo record 8261508");
        let partial = zip_path.with_extension("part");
        let client = reqwest::Client::builder()
            .user_agent("adk-playground-ap-desk")
            .build()?;
        let response = client.get(ZIP_URL).send().await?.error_for_status()?;
        let mut out = std::fs::File::create(&partial)?;
        let mut hasher = Sha256::new();
        let mut size = 0u64;
        let mut body = response.bytes_stream();
        while let Some(chunk) = body.next().await {
            let chunk = chunk?;
            size += chunk.len() as u64;
            hasher.update(&chunk);
            out.write_all(&chunk)?;
        }
        out.flush()?;
        let digest = hex::encode(hasher.finalize());
        if size != ZIP_SIZE || digest != ZIP_SHA256 {
            std::fs::remove_file(&partial).ok();
            bail!(
                "FATURA.zip arrived as {size} bytes with sha256 {digest}; the record has \
                 {ZIP_SIZE} bytes and {ZIP_SHA256}. Nothing was kept; run fetch again"
            );
        }
        std::fs::rename(&partial, &zip_path)?;
    } else {
        println!("  ok       FATURA.zip {ZIP_SIZE} bytes, sha256 checked");
    }
    let books = build(&zip_path, &inbox_dir(root))?;
    std::fs::write(dir.join("books.json"), serde_json::to_vec_pretty(&books)?)?;
    let exceptions = books
        .expected
        .iter()
        .filter(|e| e.decision != Decision::Post)
        .count();
    println!(
        "  wrote    {} documents to data/inbox ({} to post, {exceptions} exceptions), the ERP and the expected answers",
        books.expected.len(),
        books.expected.len() - exceptions
    );
    Ok(())
}

fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

/// One invoice of the batch as FATURA has it.
struct Source {
    name: String,
    template: u32,
    jpeg: Vec<u8>,
    header: Header,
}

fn build(zip_path: &Path, inbox: &Path) -> anyhow::Result<Books> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(zip_path)?)?;
    let split = read_entry(&mut zip, "invoices_dataset_final/strat1_test.csv")?;
    let split = String::from_utf8(split)?;
    let mut sources = Vec::new();
    for template in TEMPLATES {
        let mut instances: Vec<u32> = split
            .lines()
            .filter_map(|line| line.split(',').next())
            .filter_map(|image| image.strip_prefix(&format!("Template{template}_Instance")))
            .filter_map(|rest| rest.strip_suffix(".jpg"))
            .filter_map(|n| n.parse().ok())
            .collect();
        instances.sort_unstable();
        for instance in instances.into_iter().take(PER_TEMPLATE) {
            let name = format!("Template{template}_Instance{instance}");
            let annotation: serde_json::Value = serde_json::from_slice(&read_entry(
                &mut zip,
                &format!("invoices_dataset_final/Annotations/Original_Format/{name}.json"),
            )?)?;
            let jpeg = read_entry(
                &mut zip,
                &format!("invoices_dataset_final/images/{name}.jpg"),
            )?;
            let header = header_of(&annotation).with_context(|| format!("{name}'s header"))?;
            sources.push(Source {
                name,
                template,
                jpeg,
                header,
            });
        }
    }

    // The inbox interleaves the suppliers, as a day's mail does; every fourth document from
    // the sixth on is a phone photo; four invoices come again, scanned, later in the day.
    let mut order: Vec<usize> = Vec::new();
    for round in 0..PER_TEMPLATE {
        for (t, _) in TEMPLATES.iter().enumerate() {
            order.push(t * PER_TEMPLATE + round);
        }
    }
    let resent_from = [2usize, 7, 13, 18];
    std::fs::create_dir_all(inbox)?;
    let mut expected = Vec::new();
    let mut slots: Vec<(usize, Arrival)> = Vec::new();
    for (i, &source) in order.iter().enumerate() {
        let arrival = if i >= 5 && i % 4 == 1 {
            Arrival::Phone
        } else {
            Arrival::Clean
        };
        slots.push((source, arrival));
        if let Some(k) = resent_from.iter().position(|&r| r + 17 == i) {
            slots.push((order[resent_from[k]], Arrival::Resent));
        }
    }
    // Document numbers follow arrival; a re-sent copy points at its original's number.
    let doc_of_order: BTreeMap<usize, String> = slots
        .iter()
        .enumerate()
        .filter(|(_, (_, arrival))| *arrival != Arrival::Resent)
        .map(|(slot, (source, _))| (*source, format!("d{:02}", slot + 1)))
        .collect();
    let erp = erp_for(&sources);
    for (n, (source, arrival)) in slots.iter().enumerate() {
        let doc = format!("d{:02}", n + 1);
        let s = &sources[*source];
        let pixels = image::load_from_memory(&s.jpeg)?;
        let bytes = match arrival {
            Arrival::Clean => s.jpeg.clone(),
            Arrival::Phone => degrade(&pixels, n as u64, 1.6, 1.0, 12, 1.0, 40)?,
            Arrival::Resent => degrade(&pixels, n as u64, 0.7, 0.5, 6, 1.0, 70)?,
        };
        std::fs::write(inbox.join(format!("{doc}.jpg")), bytes)?;
        let duplicate_of = (*arrival == Arrival::Resent).then(|| doc_of_order[source].clone());
        let decision = decide(&erp, &s.header, duplicate_of.is_some());
        expected.push(Expected {
            doc,
            source: s.name.clone(),
            template: s.template,
            arrival: *arrival,
            duplicate_of,
            header: s.header.clone(),
            decision,
        });
    }
    Ok(Books { expected, erp })
}

fn read_entry(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> anyhow::Result<Vec<u8>> {
    let mut entry = zip
        .by_name(name)
        .with_context(|| format!("{name} is not in FATURA.zip"))?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The books the batch is matched against: every supplier but one approved; a PO for each PO
/// invoice, three of them for less than was invoiced and two with no goods receipt yet.
fn erp_for(sources: &[Source]) -> Erp {
    let mut erp = Erp::default();
    let mut po_seen = 0usize;
    for (i, s) in sources.iter().enumerate() {
        if s.template != UNAPPROVED_TEMPLATE {
            erp.vendors
                .entry(s.header.supplier.clone())
                .or_insert_with(|| Vendor {
                    id: format!("V{:03}", 100 + s.template),
                    currency: s.header.currency.clone(),
                });
        }
        if let Some(po) = &s.header.po {
            let short = matches!(po_seen, 1 | 6 | 11);
            let unreceived = matches!(po_seen, 3 | 9);
            po_seen += 1;
            let amount = if short {
                (s.header.total * 0.88 * 100.0).round() / 100.0
            } else {
                s.header.total
            };
            erp.purchase_orders.insert(
                po_key(&s.header.supplier, po),
                PurchaseOrder {
                    number: po.clone(),
                    supplier: s.header.supplier.clone(),
                    amount,
                    currency: s.header.currency.clone(),
                    goods_receipt: (!unreceived).then(|| format!("GR-{:05}", 40_000 + i)),
                },
            );
        }
    }
    erp
}

/// The decision the desk's policy gives an invoice.
fn decide(erp: &Erp, h: &Header, duplicate: bool) -> Decision {
    if duplicate {
        return Decision::Duplicate;
    }
    if !erp.vendors.contains_key(&h.supplier) {
        return Decision::UnknownSupplier;
    }
    match h
        .po
        .as_ref()
        .and_then(|po| erp.purchase_orders.get(&po_key(&h.supplier, po)))
    {
        Some(po) if h.total > po.amount * (1.0 + PO_TOLERANCE) => Decision::OverPo,
        Some(po) if po.goods_receipt.is_none() => Decision::NotReceived,
        Some(_) => Decision::Post,
        None if h.total > NON_PO_LIMIT => Decision::OverLimit,
        None => Decision::Post,
    }
}

/// A scan of the page: rotated by up to `tilt` degrees, blurred, noisy, rescaled and
/// compressed, from a seed so every fetch makes the same file.
fn degrade(
    page: &DynamicImage,
    seed: u64,
    tilt: f32,
    blur: f32,
    noise: i32,
    scale: f32,
    quality: u8,
) -> anyhow::Result<Vec<u8>> {
    let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let angle = ((next() % 2001) as f32 / 1000.0 - 1.0) * tilt.to_radians();
    let rgb = page.to_rgb8();
    let (w, h) = rgb.dimensions();
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let (sin, cos) = angle.sin_cos();
    let rotated: RgbImage = ImageBuffer::from_fn(w, h, |x, y| {
        let (dx, dy) = (x as f32 - cx, y as f32 - cy);
        let (sx, sy) = (cos * dx + sin * dy + cx, -sin * dx + cos * dy + cy);
        if sx < 0.0 || sy < 0.0 || sx >= (w - 1) as f32 || sy >= (h - 1) as f32 {
            return Rgb([246, 244, 238]);
        }
        let (x0, y0) = (sx.floor() as u32, sy.floor() as u32);
        let (fx, fy) = (sx - x0 as f32, sy - y0 as f32);
        let p = |x: u32, y: u32| rgb.get_pixel(x, y).0;
        let (a, b, c, d) = (p(x0, y0), p(x0 + 1, y0), p(x0, y0 + 1), p(x0 + 1, y0 + 1));
        Rgb(std::array::from_fn(|i| {
            let top = f32::from(a[i]) * (1.0 - fx) + f32::from(b[i]) * fx;
            let bottom = f32::from(c[i]) * (1.0 - fx) + f32::from(d[i]) * fx;
            (top * (1.0 - fy) + bottom * fy).round() as u8
        }))
    });
    let mut img = DynamicImage::ImageRgb8(rotated).blur(blur);
    if scale < 1.0 {
        let (sw, sh) = ((w as f32 * scale) as u32, (h as f32 * scale) as u32);
        img = img.resize_exact(sw, sh, image::imageops::FilterType::Triangle);
    }
    let mut rgb = img.to_rgb8();
    for pixel in rgb.pixels_mut() {
        let shift = (next() % (2 * noise as u64 + 1)) as i32 - noise;
        for c in &mut pixel.0 {
            *c = (i32::from(*c) + shift - 6).clamp(0, 255) as u8;
        }
    }
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
        .encode_image(&DynamicImage::ImageRgb8(rgb))?;
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Normalising fields
// ---------------------------------------------------------------------------------------------

fn text<'a>(annotation: &'a serde_json::Value, field: &str) -> Option<&'a str> {
    annotation.get(field)?.get("text")?.as_str()
}

/// The header the annotations give, normalised as the desk compares fields.
fn header_of(a: &serde_json::Value) -> anyhow::Result<Header> {
    let supplier = text(a, "SELLER_NAME")
        .context("no SELLER_NAME")?
        .trim()
        .to_string();
    let number = invoice_number(text(a, "NUMBER").context("no NUMBER")?);
    let date = date(text(a, "DATE").context("no DATE")?).context("an unreadable DATE")?;
    let po = text(a, "PO_NUMBER").map(|t| t.rsplit(':').next().unwrap_or(t).trim().to_string());
    let total_text = text(a, "TOTAL").context("no TOTAL")?;
    let (total, currency) = amount(total_text).context("an unreadable TOTAL")?;
    Ok(Header {
        supplier,
        number,
        date,
        po,
        total,
        currency,
    })
}

/// `INVOICE ID INV/80-94/770`, `INVOICE # 2533-604`, `Invoice number INV47355637` → the id.
pub fn invoice_number(text: &str) -> String {
    let lower = text.to_lowercase();
    let mut rest = text;
    for label in [
        "invoice number",
        "invoice id",
        "invoice no.",
        "invoice no",
        "invoice #",
        "invoice",
    ] {
        if let Some(at) = lower.find(label) {
            rest = &text[at + label.len()..];
            break;
        }
    }
    rest.trim_start_matches([':', '#', ' ', '.'])
        .trim()
        .to_string()
}

/// `Invoice Date: 06-Jun-2013` → `2013-06-06`.
pub fn date(text: &str) -> Option<String> {
    let value = text.rsplit(':').next()?.trim();
    let parsed = chrono::NaiveDate::parse_from_str(value, "%d-%b-%Y")
        .or_else(|_| chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d"))
        .or_else(|_| chrono::NaiveDate::parse_from_str(value, "%d/%m/%Y"))
        .ok()?;
    Some(parsed.format("%Y-%m-%d").to_string())
}

/// `TOTAL : 602.70 $`, `BALANCE DUE : 1203.25 EUR` → the amount and an ISO currency.
pub fn amount(text: &str) -> Option<(f64, String)> {
    let number = text
        .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
        .rfind(|t| t.contains('.'))?
        .replace(',', "");
    let value: f64 = number.parse().ok()?;
    let upper = text.to_uppercase();
    let currency = if upper.contains("EUR") || text.contains('€') {
        "EUR"
    } else if upper.contains("GBP") || text.contains('£') {
        "GBP"
    } else {
        "USD"
    };
    Some((value, currency.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_normalise_as_the_annotations_print_them() {
        assert_eq!(invoice_number("INVOICE ID INV/80-94/770"), "INV/80-94/770");
        assert_eq!(invoice_number("INVOICE # 2533-604"), "2533-604");
        assert_eq!(invoice_number("Invoice number INV47355637"), "INV47355637");
        assert_eq!(
            date("Invoice Date: 06-Jun-2013").as_deref(),
            Some("2013-06-06")
        );
        assert_eq!(amount("TOTAL : 602.70 $"), Some((602.70, "USD".into())));
        assert_eq!(
            amount("BALANCE DUE : 1203.25 EUR"),
            Some((1203.25, "EUR".into()))
        );
        assert_eq!(
            amount("TOTAL : 1,107.09 USD"),
            Some((1107.09, "USD".into()))
        );
    }
}
