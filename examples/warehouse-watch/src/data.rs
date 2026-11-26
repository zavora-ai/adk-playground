//! The dataset: NVIDIA PhysicalAI-SmartSpaces, `MTMC_Tracking_2026/train/Warehouse_000`
//! (CC BY 4.0), fetched by revision with every file's size and SHA-256 checked, the cameras'
//! calibration, and the ground truth cut down to what the measurement needs.

use std::collections::HashMap;
use std::fmt;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use futures::StreamExt;
use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

/// The Hugging Face dataset and the commit every file is fetched at.
pub const REPO: &str = "nvidia/PhysicalAI-SmartSpaces";
pub const REVISION: &str = "d38193be60906679124732a4be9a6516821e2bed";
pub const SCENE: &str = "MTMC_Tracking_2026/train/Warehouse_000";

/// The four cameras the app watches: the ones that see most of the scene's close approaches
/// between a moving forklift and a person.
pub const CAMERAS: [&str; 4] = ["Camera_0001", "Camera_0003", "Camera_0005", "Camera_0015"];

/// Every frame of the videos is 1920 × 1080 at 30 fps; ground truth is per frame.
pub const VIDEO_FPS: u64 = 30;
/// Ground truth is kept at the rate the cameras are sampled: every sixth frame, 5 a second.
pub const TRUTH_STEP: u64 = 6;

/// One file of the dataset at [`REVISION`].
pub struct DataFile {
    pub path: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

impl DataFile {
    pub fn name(&self) -> &'static str {
        self.path.rsplit('/').next().unwrap_or(self.path)
    }

    pub fn url(&self) -> String {
        format!(
            "https://huggingface.co/datasets/{REPO}/resolve/{REVISION}/{SCENE}/{}",
            self.path
        )
    }
}

/// What `fetch` downloads, 1.27 GB in all. The SHA-256s are the dataset's Git LFS object ids.
pub const FILES: [DataFile; 7] = [
    DataFile {
        path: "videos/Camera_0001.mp4",
        size: 242_275_945,
        sha256: "aebf51f04434f0cccfb27d9e0385a5bde5d91a15307a91aa17f112088c6e7256",
    },
    DataFile {
        path: "videos/Camera_0003.mp4",
        size: 221_996_855,
        sha256: "7668fc49c5903174626894ceb7cc2fd30a69c362a9a2734d7e75a3f2b4eb15db",
    },
    DataFile {
        path: "videos/Camera_0005.mp4",
        size: 188_989_348,
        sha256: "85e359566e0bd4bdd0db6eaf28ac171b21e4af80f64bc461e703c7935b65e45b",
    },
    DataFile {
        path: "videos/Camera_0015.mp4",
        size: 212_738_760,
        sha256: "0092b24cfc01e716784f9e302ad5b08f9dd9509a8a60414e67f2d9d7fcb2d2ed",
    },
    DataFile {
        path: "calibration.json",
        size: 44_111,
        sha256: "93091b6e4a47600075a8331135982d23e1f44d5a9abb5fb2959cbe44216257c5",
    },
    DataFile {
        path: "ground_truth.json",
        size: 281_636_586,
        sha256: "0c160199a14f04bcb95e2ece7c9c3be37f1ffa29a4b1fd1a390c607d21c28109",
    },
    DataFile {
        path: "map.png",
        size: 1_123_636,
        sha256: "0bb7fd5915654b6fa4a1e984d9ed4d83eec41a7ebab8a045badf16a40348890a",
    },
];

/// Where the fetched files live.
pub fn data_dir(root: &Path) -> PathBuf {
    root.join("data")
}

pub fn video(root: &Path, camera: &str) -> PathBuf {
    data_dir(root).join(format!("{camera}.mp4"))
}

pub fn truth_path(root: &Path) -> PathBuf {
    data_dir(root).join("truth.json")
}

/// Downloads every file not already present with the right digest, checking size and
/// SHA-256 as it streams, then writes the cut-down ground truth.
pub async fn fetch(root: &Path) -> anyhow::Result<()> {
    let dir = data_dir(root);
    std::fs::create_dir_all(&dir)?;
    let client = reqwest::Client::builder()
        .user_agent("adk-playground-warehouse-watch")
        .build()?;
    for file in &FILES {
        let target = dir.join(file.name());
        if target.exists() && sha256_file(&target)? == file.sha256 {
            println!(
                "  ok       {:<20} {:>12} bytes, sha256 checked",
                file.name(),
                file.size
            );
            continue;
        }
        println!(
            "  fetching {:<20} {:>12} bytes from {REPO}@{}",
            file.name(),
            file.size,
            &REVISION[..10]
        );
        let partial = target.with_extension("part");
        let response = client.get(file.url()).send().await?.error_for_status()?;
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
        if size != file.size || digest != file.sha256 {
            std::fs::remove_file(&partial).ok();
            bail!(
                "{} arrived as {size} bytes with sha256 {digest}; the pinned revision has {} \
                 bytes and {}. Nothing was kept; run fetch again",
                file.name(),
                file.size,
                file.sha256
            );
        }
        std::fs::rename(&partial, &target)?;
    }
    println!("  cutting ground_truth.json to 5 frames a second for the four cameras…");
    let truth = Truth::from_dataset(&dir.join("ground_truth.json"))?;
    std::fs::write(truth_path(root), serde_json::to_vec(&truth)?)?;
    println!(
        "  wrote    truth.json: {} samples, {} objects",
        truth.samples.len(),
        truth.samples.first().map_or(0, |s| s.objects.len())
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_floor_and_the_image_invert_each_other() {
        // Camera_0015's homography from the dataset.
        let floor = Floor::new([
            [113.45623816574759, -125.58176893074133, -6615.81163721184],
            [22.039762259823107, 4.737279261522137, -105.86947776036769],
            [0.08522716510429507, 0.018313423863988695, 1.0],
        ])
        .unwrap();
        let (u, v) = floor.image(-62.5, -59.3);
        let (x, y) = floor.floor(u, v);
        assert!(
            (x + 62.5).abs() < 1e-6 && (y + 59.3).abs() < 1e-6,
            "{x}, {y}"
        );
    }

    #[test]
    fn a_footprint_gap_is_zero_inside_and_measured_from_the_edge() {
        let forklift = TruthObject {
            kind: Kind::Forklift,
            id: 1,
            x: 0.0,
            y: 0.0,
            sx: 1.0,
            sy: 2.0,
            yaw: 0.0,
            cameras: 1,
            boxes: [None; 4],
        };
        assert_eq!(forklift.gap(0.2, 0.5), 0.0);
        assert!((forklift.gap(1.5, 0.0) - 1.0).abs() < 1e-9);
        assert!((forklift.gap(0.0, -2.0) - 1.0).abs() < 1e-9);
    }
}

fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

/// Checks every fetched file is present with its pinned size, before a run reads them.
pub fn check(root: &Path) -> anyhow::Result<()> {
    for file in &FILES {
        let path = data_dir(root).join(file.name());
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if size != file.size {
            bail!(
                "{} is missing or incomplete; run `cargo run --release -- fetch` first",
                path.display()
            );
        }
    }
    if !truth_path(root).exists() {
        bail!("data/truth.json is missing; run `cargo run --release -- fetch` again");
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Calibration
// ---------------------------------------------------------------------------------------------

/// A camera's ground-plane homography: floor metres `(x, y, 1)` to image pixels of the
/// 1920 × 1080 frame.
#[derive(Clone, Debug)]
pub struct Floor {
    to_image: [[f64; 3]; 3],
    to_floor: [[f64; 3]; 3],
}

impl Floor {
    pub fn new(to_image: [[f64; 3]; 3]) -> anyhow::Result<Self> {
        let to_floor = invert(&to_image).context("the camera's homography is singular")?;
        Ok(Self { to_image, to_floor })
    }

    /// The floor point under image pixel `(u, v)` of the full frame.
    pub fn floor(&self, u: f64, v: f64) -> (f64, f64) {
        apply(&self.to_floor, u, v)
    }

    /// The image pixel of floor point `(x, y)`.
    pub fn image(&self, x: f64, y: f64) -> (f64, f64) {
        apply(&self.to_image, x, y)
    }
}

fn apply(m: &[[f64; 3]; 3], a: f64, b: f64) -> (f64, f64) {
    let w = m[2][0] * a + m[2][1] * b + m[2][2];
    (
        (m[0][0] * a + m[0][1] * b + m[0][2]) / w,
        (m[1][0] * a + m[1][1] * b + m[1][2]) / w,
    )
}

fn invert(m: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let c = |r: usize, k: usize| m[r % 3][k % 3];
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = c(j + 1, i + 1) * c(j + 2, i + 2) - c(j + 1, i + 2) * c(j + 2, i + 1);
        }
    }
    let det = m[0][0] * out[0][0] + m[0][1] * out[1][0] + m[0][2] * out[2][0];
    if det.abs() < 1e-12 {
        return None;
    }
    for row in &mut out {
        for cell in row.iter_mut() {
            *cell /= det;
        }
    }
    Some(out)
}

/// The four cameras' floors, from the dataset's `calibration.json`.
pub fn floors(root: &Path) -> anyhow::Result<HashMap<String, Floor>> {
    #[derive(Deserialize)]
    struct Sensor {
        id: String,
        homography: [[f64; 3]; 3],
    }
    #[derive(Deserialize)]
    struct Calibration {
        sensors: Vec<Sensor>,
    }
    let text = std::fs::read_to_string(data_dir(root).join("calibration.json"))?;
    let calibration: Calibration = serde_json::from_str(&text)?;
    let mut floors = HashMap::new();
    for sensor in calibration.sensors {
        if CAMERAS.contains(&sensor.id.as_str()) {
            floors.insert(sensor.id.clone(), Floor::new(sensor.homography)?);
        }
    }
    Ok(floors)
}

// ---------------------------------------------------------------------------------------------
// Ground truth
// ---------------------------------------------------------------------------------------------

/// What the measurement needs of the ground truth: every object's floor position and
/// footprint at 5 frames a second, and which of the four cameras show it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Truth {
    pub samples: Vec<Sample>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sample {
    /// The video frame, 30 a second from 0.
    pub frame: u64,
    pub objects: Vec<TruthObject>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Person,
    Forklift,
    PalletTruck,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TruthObject {
    pub kind: Kind,
    pub id: u64,
    /// Floor position of the object's centre, metres.
    pub x: f64,
    pub y: f64,
    /// Footprint length along the object's x and y axes, metres, and its heading, radians.
    pub sx: f64,
    pub sy: f64,
    pub yaw: f64,
    /// Bit `i` set when `CAMERAS[i]` shows the object.
    pub cameras: u8,
    /// The visible part's box in each of `CAMERAS`, in 1920 × 1080 pixels.
    pub boxes: [Option<[f32; 4]>; 4],
}

impl TruthObject {
    /// The distance from `(x, y)` to this object's footprint, 0 inside it.
    pub fn gap(&self, x: f64, y: f64) -> f64 {
        let (dx, dy) = (x - self.x, y - self.y);
        let (c, s) = ((-self.yaw).cos(), (-self.yaw).sin());
        let (lx, ly) = (dx * c - dy * s, dx * s + dy * c);
        let ex = (lx.abs() - self.sx / 2.0).max(0.0);
        let ey = (ly.abs() - self.sy / 2.0).max(0.0);
        ex.hypot(ey)
    }
}

/// One object as `ground_truth.json` writes it; keys hold spaces.
#[derive(Deserialize)]
struct RawObject {
    #[serde(rename = "object type")]
    kind: String,
    #[serde(rename = "object id")]
    id: u64,
    #[serde(rename = "3d location")]
    location: [f64; 3],
    #[serde(rename = "3d bounding box scale")]
    scale: [f64; 3],
    #[serde(rename = "3d bounding box rotation")]
    rotation: [f64; 3],
    #[serde(rename = "2d bounding box visible")]
    visible: HashMap<String, [f64; 4]>,
}

impl Truth {
    /// Reads the 282 MB file once, keeping every sixth frame and skipping the rest unparsed.
    pub fn from_dataset(path: &Path) -> anyhow::Result<Self> {
        struct Frames;
        impl<'de> Visitor<'de> for Frames {
            type Value = Vec<Sample>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a map of frame numbers to objects")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut samples = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    let frame: u64 = key.parse().map_err(serde::de::Error::custom)?;
                    if !frame.is_multiple_of(TRUTH_STEP) {
                        map.next_value::<IgnoredAny>()?;
                        continue;
                    }
                    let raw: Vec<RawObject> = map.next_value()?;
                    let objects = raw
                        .into_iter()
                        .filter_map(|o| {
                            let kind = match o.kind.as_str() {
                                "Person" => Kind::Person,
                                "Forklift" => Kind::Forklift,
                                "PalletTruck" => Kind::PalletTruck,
                                _ => return None,
                            };
                            let boxes = CAMERAS.map(|c| {
                                o.visible
                                    .get(c)
                                    .filter(|b| (b[2] - b[0]) * (b[3] - b[1]) > 0.0)
                                    .map(|b| b.map(|v| v as f32))
                            });
                            let cameras = boxes.iter().enumerate().fold(0u8, |mask, (i, b)| {
                                if b.is_some() { mask | (1 << i) } else { mask }
                            });
                            Some(TruthObject {
                                kind,
                                id: o.id,
                                x: o.location[0],
                                y: o.location[1],
                                sx: o.scale[0],
                                sy: o.scale[1],
                                yaw: o.rotation[2],
                                cameras,
                                boxes,
                            })
                        })
                        .collect();
                    samples.push(Sample { frame, objects });
                }
                samples.sort_by_key(|s| s.frame);
                Ok(samples)
            }
        }
        struct Root(Vec<Sample>);
        impl<'de> Deserialize<'de> for Root {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                d.deserialize_map(Frames).map(Root)
            }
        }
        let reader = BufReader::with_capacity(1 << 20, std::fs::File::open(path)?);
        let Root(samples) = serde_json::from_reader(reader)?;
        Ok(Self { samples })
    }

    pub fn load(root: &Path) -> anyhow::Result<Self> {
        Ok(serde_json::from_slice(&std::fs::read(truth_path(root))?)?)
    }
}
