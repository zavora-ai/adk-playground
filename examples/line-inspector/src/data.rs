//! The dataset: VisA's candle class (CC BY 4.0), fetched as the first 129 MB of the VisA
//! archive and checked by SHA-256, and the trays of the bank, the calibration and the line.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The VisA archive on Amazon's open-data bucket; candle is its first class.
pub const ARCHIVE_URL: &str =
    "https://amazon-visual-anomaly.s3.us-west-2.amazonaws.com/VisA_20220922.tar";
/// Candle's bytes: from the archive's start to the first entry of the next class.
pub const CANDLE_BYTES: u64 = 129_238_016;
pub const CANDLE_SHA256: &str = "57d2c00c6ff1255de2372d373edcd066b7802df117e131b48af0e393fbdfae15";

/// Good trays whose candles form the bank of known-good tiles.
pub const BANK: std::ops::Range<u32> = 0..40;
/// Good trays that set the pass and reject thresholds.
pub const CALIBRATION: std::ops::Range<u32> = 100..130;
/// The line: forty trays with a defect and forty without, interleaved.
pub const LINE_ANOMALY: std::ops::Range<u32> = 0..40;
pub const LINE_NORMAL: std::ops::Range<u32> = 900..940;

pub fn data_dir(root: &Path) -> PathBuf {
    root.join("data")
}

pub fn candle_dir(root: &Path) -> PathBuf {
    data_dir(root).join("candle")
}

pub fn normal_image(root: &Path, n: u32) -> PathBuf {
    candle_dir(root).join(format!("Data/Images/Normal/{n:04}.JPG"))
}

/// One tray on the line, in running order.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tray {
    /// `t001` …: the tray's place on the line.
    pub id: String,
    pub image: PathBuf,
    /// The defect mask, for a tray with a defect.
    pub mask: Option<PathBuf>,
    /// VisA's name for the defect.
    pub defect: Option<String>,
}

/// The line's trays: anomaly and normal trays interleaved by a fixed shuffle.
pub fn line(root: &Path) -> anyhow::Result<Vec<Tray>> {
    let dir = candle_dir(root);
    let anno = std::fs::read_to_string(dir.join("image_anno.csv"))
        .context("run `cargo run --release -- fetch` first")?;
    let defect_of = |n: u32| -> Option<String> {
        let key = format!("Anomaly/{n:03}.JPG");
        anno.lines().find(|l| l.contains(&key)).map(|l| {
            let fields: Vec<&str> = l.split(',').collect();
            fields[1..fields.len() - 1]
                .join(",")
                .trim_matches('"')
                .trim()
                .to_string()
        })
    };
    let mut items: Vec<(u64, Tray)> = Vec::new();
    for n in LINE_ANOMALY {
        items.push((
            mix(u64::from(n) * 2 + 1),
            Tray {
                id: String::new(),
                image: dir.join(format!("Data/Images/Anomaly/{n:03}.JPG")),
                mask: Some(dir.join(format!("Data/Masks/Anomaly/{n:03}.png"))),
                defect: defect_of(n),
            },
        ));
    }
    for n in LINE_NORMAL {
        items.push((
            mix(u64::from(n) * 2),
            Tray {
                id: String::new(),
                image: normal_image(root, n),
                mask: None,
                defect: None,
            },
        ));
    }
    items.sort_by_key(|(k, _)| *k);
    Ok(items
        .into_iter()
        .enumerate()
        .map(|(i, (_, mut tray))| {
            tray.id = format!("t{:03}", i + 1);
            tray
        })
        .collect())
}

fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Downloads candle's byte range of the VisA archive, checks its size and SHA-256, and
/// unpacks it into `data/candle`.
pub async fn fetch(root: &Path) -> anyhow::Result<()> {
    let dir = data_dir(root);
    std::fs::create_dir_all(&dir)?;
    let tar = dir.join("candle.tar");
    if !(tar.exists() && sha256_file(&tar)? == CANDLE_SHA256) {
        println!(
            "  fetching candle: bytes 0–{} of VisA_20220922.tar",
            CANDLE_BYTES - 1
        );
        let partial = tar.with_extension("part");
        let client = reqwest::Client::builder()
            .user_agent("adk-playground-line-inspector")
            .build()?;
        let response = client
            .get(ARCHIVE_URL)
            .header("Range", format!("bytes=0-{}", CANDLE_BYTES - 1))
            .send()
            .await?
            .error_for_status()?;
        let mut out = std::fs::File::create(&partial)?;
        let mut hasher = Sha256::new();
        let mut size = 0u64;
        let mut body = response.bytes_stream();
        while let Some(chunk) = body.next().await {
            let chunk = chunk?;
            let take = chunk.len().min((CANDLE_BYTES - size) as usize);
            hasher.update(&chunk[..take]);
            out.write_all(&chunk[..take])?;
            size += take as u64;
            if size == CANDLE_BYTES {
                break;
            }
        }
        out.flush()?;
        let digest = hex::encode(hasher.finalize());
        if size != CANDLE_BYTES || digest != CANDLE_SHA256 {
            std::fs::remove_file(&partial).ok();
            bail!(
                "candle arrived as {size} bytes with sha256 {digest}; expected {CANDLE_BYTES} bytes and \
                 {CANDLE_SHA256}. Nothing was kept; run fetch again"
            );
        }
        std::fs::rename(&partial, &tar)?;
    } else {
        println!("  ok       candle.tar {CANDLE_BYTES} bytes, sha256 checked");
    }
    let mut archive = tar::Archive::new(std::fs::File::open(&tar)?);
    let mut files = 0;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if !path.starts_with("candle") || entry.header().entry_type().is_dir() {
            continue;
        }
        let target = dir.join(&path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        std::fs::write(target, bytes)?;
        files += 1;
    }
    println!("  unpacked {files} files: 1,000 good trays, 100 with a defect and their masks");
    Ok(())
}

fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}
