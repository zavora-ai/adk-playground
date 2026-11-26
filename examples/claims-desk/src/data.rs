//! The claims: photos curated from Wikimedia Commons — each file free to reuse, its licence
//! and author recorded, its severity from the Commons category it is filed in — fetched by
//! size and SHA-1, and nineteen first notices of loss written around them.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use image::imageops::FilterType;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

/// Severity as Commons files the photo: minor, moderate or major crash damage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Minor,
    Moderate,
    Major,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minor => "minor",
            Self::Moderate => "moderate",
            Self::Major => "major",
        }
    }
}

/// One Commons file, pinned by size and SHA-1 as Commons reports them.
pub struct Source {
    pub key: &'static str,
    pub title: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha1: &'static str,
    pub licence: &'static str,
    pub artist: &'static str,
    /// The Commons category's severity.
    pub severity: Severity,
}

pub const SOURCES: [Source; 20] = [
    Source {
        key: "suzuki1",
        title: "Abandoned Suzuki, Osidge, London.jpg",
        url: "https://upload.wikimedia.org/wikipedia/commons/5/56/Abandoned_Suzuki%2C_Osidge%2C_London.jpg",
        size: 3860860,
        sha1: "1b8c6c9bd44399e66080c37c1286a89ccd7cf75e",
        licence: "CC BY-SA 4.0",
        artist: "Philafrenzy",
        severity: Severity::Minor,
    },
    Source {
        key: "suzuki2",
        title: "Abandoned Suzuki, Osidge, London (2).jpg",
        url: "https://upload.wikimedia.org/wikipedia/commons/4/41/Abandoned_Suzuki%2C_Osidge%2C_London_%282%29.jpg",
        size: 4323583,
        sha1: "e5b31cc50d2f20b54f0fef75398611fcec299803",
        licence: "CC BY-SA 4.0",
        artist: "Philafrenzy",
        severity: Severity::Minor,
    },
    Source {
        key: "dodge",
        title: "US Government wrecked Dodge Stratus in Manhattan, 2005.jpg",
        url: "https://upload.wikimedia.org/wikipedia/commons/2/24/US_Government_wrecked_Dodge_Stratus_in_Manhattan%2C_2005.jpg",
        size: 1558338,
        sha1: "7667a3594d17b8160a76a3f19d9316a4e4e89425",
        licence: "CC BY 2.0",
        artist: "Diego Torres Silvestre from Sao Paulo, Brazil",
        severity: Severity::Minor,
    },
    Source {
        key: "hhr",
        title: "2007 Chevrolet HHR involved in crash.jpg",
        url: "https://upload.wikimedia.org/wikipedia/commons/0/03/2007_Chevrolet_HHR_involved_in_crash.jpg",
        size: 327901,
        sha1: "2aa68ff03f3090a94457e57e79e7e454f849807c",
        licence: "CC0",
        artist: "Crazytales",
        severity: Severity::Moderate,
    },
    Source {
        key: "civic",
        title: "90e Avenue, LaSalle - 10.jpg",
        url: "https://upload.wikimedia.org/wikipedia/commons/7/76/90e_Avenue%2C_LaSalle_-_10.jpg",
        size: 4079366,
        sha1: "aa093dca264cf7a7c882d9e429e975b4a2e738ca",
        licence: "CC BY-SA 4.0",
        artist: "Jeangagnon",
        severity: Severity::Moderate,
    },
    Source {
        key: "sonic",
        title: "90e Avenue, LaSalle - 12.jpg",
        url: "https://upload.wikimedia.org/wikipedia/commons/b/b5/90e_Avenue%2C_LaSalle_-_12.jpg",
        size: 3931781,
        sha1: "e6ab08feb23d8b7b19d51c426ea004b68beb8ecb",
        licence: "CC BY-SA 4.0",
        artist: "Jeangagnon",
        severity: Severity::Moderate,
    },
    Source {
        key: "w211",
        title: "Crashed black W211.jpg",
        url: "https://upload.wikimedia.org/wikipedia/commons/5/51/Crashed_black_W211.jpg",
        size: 872777,
        sha1: "e22ba45120e14b1c0a2c31641d73154fc35d3327",
        licence: "CC BY-SA 3.0",
        artist: "Matti Blume",
        severity: Severity::Moderate,
    },
    Source {
        key: "fabia",
        title: "Fabia I (2003) accident front right.jpg",
        url: "https://upload.wikimedia.org/wikipedia/commons/9/97/Fabia_I_%282003%29_accident_front_right.jpg",
        size: 212031,
        sha1: "4e2bb1423e3415be0bdcdf27ffbac5dbe987e0a6",
        licence: "CC BY-SA 4.0",
        artist: "Hundehalter",
        severity: Severity::Moderate,
    },
    Source {
        key: "slk",
        title: "Front crashed SLK.jpg",
        url: "https://upload.wikimedia.org/wikipedia/commons/9/90/Front_crashed_SLK.jpg",
        size: 536409,
        sha1: "2a6470026450a7d0462fa5876e1fe60f9f25e8be",
        licence: "CC BY-SA 3.0",
        artist: "Matti Blume (MB-one)",
        severity: Severity::Moderate,
    },
    Source {
        key: "c5",
        title: "Poor Citroen C5... (44990379005).jpg",
        url: "https://upload.wikimedia.org/wikipedia/commons/c/cd/Poor_Citroen_C5..._%2844990379005%29.jpg",
        size: 1101146,
        sha1: "4eb7ac29f6f50778e0ff90931cf6da003bff695b",
        licence: "CC0",
        artist: "Guillaume Vachey from Chalon sur Saone, France",
        severity: Severity::Moderate,
    },
    Source {
        key: "levorg",
        title: "Subaru Levorg (48028046188).jpg",
        url: "https://upload.wikimedia.org/wikipedia/commons/6/65/Subaru_Levorg_%2848028046188%29.jpg",
        size: 2122772,
        sha1: "d9548e6d2a5eafdeab69e007e740fbdbb5bfcbd4",
        licence: "CC0",
        artist: "Guillaume Vachey from Chalon sur Saone, France",
        severity: Severity::Moderate,
    },
    Source {
        key: "sprinter",
        title: "90e Avenue, LaSalle - 36.jpg",
        url: "https://upload.wikimedia.org/wikipedia/commons/2/23/90e_Avenue%2C_LaSalle_-_36.jpg",
        size: 3650036,
        sha1: "3c1cb7cfa6c6da7d59f734b4be70d46093a7753a",
        licence: "CC BY-SA 4.0",
        artist: "Jeangagnon",
        severity: Severity::Major,
    },
    Source {
        key: "kia",
        title: "Broken Kia.jpg",
        url: "https://upload.wikimedia.org/wikipedia/commons/b/bd/Broken_Kia.jpg",
        size: 11997818,
        sha1: "d2a46647882d1ade50ee34f11714cc3d1890a242",
        licence: "CC BY-SA 4.0",
        artist: "Oleg Yunakov",
        severity: Severity::Major,
    },
    Source {
        key: "sclass",
        title: "Crashed black S-Class.jpg",
        url: "https://upload.wikimedia.org/wikipedia/commons/b/bf/Crashed_black_S-Class.jpg",
        size: 580969,
        sha1: "c7fbe7078a3228bde6fa3efda77d0769a2119895",
        licence: "CC BY-SA 3.0",
        artist: "Matti Blume (MB-one)",
        severity: Severity::Major,
    },
    Source {
        key: "audi",
        title: "Damaged Audi, New Barnet (2).jpg",
        url: "https://upload.wikimedia.org/wikipedia/commons/8/84/Damaged_Audi%2C_New_Barnet_%282%29.jpg",
        size: 4562700,
        sha1: "f7ac7e30094203a698e1cc7266eb42542139a893",
        licence: "CC BY-SA 4.0",
        artist: "Philafrenzy",
        severity: Severity::Major,
    },
    Source {
        key: "lupo",
        title: "Damaged Volkswagen Lupo in Jura, France (2018) 1.jpg",
        url: "https://upload.wikimedia.org/wikipedia/commons/8/82/Damaged_Volkswagen_Lupo_in_Jura%2C_France_%282018%29_1.jpg",
        size: 4979081,
        sha1: "7e630d21fbaf2a816f171ac63bcfde7b1e14bb4b",
        licence: "CC BY-SA 4.0",
        artist: "Sebleouf",
        severity: Severity::Major,
    },
    Source {
        key: "c15",
        title: "Citroen C 15 D 2001 (10431763586).jpg",
        url: "https://upload.wikimedia.org/wikipedia/commons/8/8c/Citroen_C_15_D_2001_%2810431763586%29.jpg",
        size: 623287,
        sha1: "9f4f7777dfab4f9fb082cdc48949c63a62f76320",
        licence: "CC BY-SA 2.0",
        artist: "order_242 from Chile",
        severity: Severity::Major,
    },
    Source {
        key: "typer",
        title: "Fn22.jpg",
        url: "https://upload.wikimedia.org/wikipedia/commons/3/33/Fn22.jpg",
        size: 433681,
        sha1: "148072f51a0f044b70f77c2fb691ffbf43f2cc03",
        licence: "CC BY-SA 4.0",
        artist: "Canelo 93",
        severity: Severity::Major,
    },
    Source {
        key: "impala",
        title: "Vehicle After a Side-Impact Collision.jpg",
        url: "https://upload.wikimedia.org/wikipedia/commons/e/e3/Vehicle_After_a_Side-Impact_Collision.jpg",
        size: 1302171,
        sha1: "6fcac6659c64e2343691410e8f7024266a03b807",
        licence: "CC BY 4.0",
        artist: "Kkiefuik",
        severity: Severity::Major,
    },
    Source {
        key: "cclass",
        title: "Wrecked Mercedes, north Finchley, London 01.JPG",
        url: "https://upload.wikimedia.org/wikipedia/commons/9/90/Wrecked_Mercedes%2C_north_Finchley%2C_London_01.JPG",
        size: 4446762,
        sha1: "1a212bbb27bfefe6b2c21fa3dfb9339c7a21690f",
        licence: "CC BY-SA 4.0",
        artist: "Philafrenzy",
        severity: Severity::Major,
    },
];

pub fn source(key: &str) -> &'static Source {
    SOURCES
        .iter()
        .find(|s| s.key == key)
        .expect("a source the claims name")
}

/// A photo of a claim: a Commons file as sent, or a second copy of one cropped and
/// re-compressed, as a reused photo arrives.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Photo {
    pub name: String,
    pub source: String,
    pub reused: bool,
}

/// What a claim should come to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Estimate,
    ReusedPhoto,
    DifferentVehicle,
}

/// A first notice of loss, its photos and what it should come to.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Claim {
    pub id: String,
    pub policy: String,
    pub vehicle: String,
    pub date: String,
    pub story: String,
    pub photos: Vec<Photo>,
    pub outcome: Outcome,
    /// The Commons category's severity of the claim's own photos.
    pub severity: Severity,
    /// The damaged side, where the Commons title or description names it.
    pub side: Option<String>,
}

fn claim(
    n: u32,
    vehicle: &str,
    date: &str,
    story: &str,
    photos: &[(&str, bool)],
    outcome: Outcome,
    side: Option<&str>,
) -> Claim {
    let id = format!("CL-{:04}", 400 + n);
    let severity = source(photos[0].0).severity;
    Claim {
        id: id.clone(),
        policy: format!("MP-{:06}", 318_204 + n * 7_919),
        vehicle: vehicle.into(),
        date: date.into(),
        story: story.into(),
        photos: photos
            .iter()
            .enumerate()
            .map(|(i, (key, reused))| Photo {
                name: format!("{id}-{}.jpg", i + 1),
                source: (*key).into(),
                reused: *reused,
            })
            .collect(),
        outcome,
        severity,
        side: side.map(str::to_string),
    }
}

/// The day's claims, in the order they arrive.
pub fn claims() -> Vec<Claim> {
    use Outcome::{DifferentVehicle, Estimate, ReusedPhoto};
    vec![
        claim(
            1,
            "2015 Suzuki Swift, blue",
            "2026-11-02",
            "The car was hit while parked on the street overnight; the front left wing and wheel are damaged.",
            &[("suzuki1", false), ("suzuki2", false)],
            Estimate,
            None,
        ),
        claim(
            2,
            "2005 Dodge Stratus sedan, gold",
            "2026-11-03",
            "Ran into the back of a delivery truck in slow traffic; the front end took the hit.",
            &[("dodge", false)],
            Estimate,
            None,
        ),
        claim(
            3,
            "2007 Chevrolet HHR, silver",
            "2026-11-03",
            "Hit by a car turning across the junction; the front left corner is crushed.",
            &[("hhr", false)],
            Estimate,
            None,
        ),
        claim(
            4,
            "2008 Honda Civic sedan, black",
            "2026-11-04",
            "Side-swiped in a car park; both doors on the driver's side are pushed in.",
            &[("civic", false), ("sonic", false)],
            DifferentVehicle,
            None,
        ),
        claim(
            5,
            "2005 Mercedes-Benz E-Class sedan, black",
            "2026-11-05",
            "Front-end collision on the main road; the car was towed.",
            &[("w211", false)],
            Estimate,
            None,
        ),
        claim(
            6,
            "2003 Škoda Fabia hatchback, white",
            "2026-11-05",
            "Clipped a bollard turning out of a narrow street; the front right corner is damaged.",
            &[("fabia", false)],
            Estimate,
            Some("front right"),
        ),
        claim(
            7,
            "2000 Mercedes-Benz SLK roadster, silver",
            "2026-11-06",
            "Front collision at low speed with a parked car.",
            &[("slk", false)],
            Estimate,
            Some("front"),
        ),
        claim(
            8,
            "2004 Citroën C5 hatchback, white",
            "2026-11-07",
            "Hit a kerb post leaving a car park; damage at the front right.",
            &[("c5", false)],
            Estimate,
            None,
        ),
        claim(
            9,
            "2018 Subaru Levorg estate, blue",
            "2026-11-07",
            "Front right corner hit by a reversing van.",
            &[("levorg", false)],
            Estimate,
            None,
        ),
        claim(
            10,
            "2014 Mercedes-Benz Sprinter van, white",
            "2026-11-08",
            "Collided with a lorry at night; the van was towed to the yard.",
            &[("sprinter", false)],
            Estimate,
            None,
        ),
        claim(
            11,
            "2014 Kia Optima sedan, silver",
            "2026-11-09",
            "Head-on collision at an intersection; the airbags went off.",
            &[("kia", false)],
            Estimate,
            None,
        ),
        claim(
            12,
            "2002 Mercedes-Benz S-Class sedan, black",
            "2026-11-10",
            "Front right impact with a wall; the car is not drivable.",
            &[("sclass", false)],
            Estimate,
            None,
        ),
        claim(
            13,
            "2016 Audi A3 Sportback, black",
            "2026-11-11",
            "Lost control on a wet road and hit a lamp post with the front.",
            &[("audi", false)],
            Estimate,
            None,
        ),
        claim(
            14,
            "2001 Volkswagen Lupo hatchback, white",
            "2026-11-12",
            "Front collision on a country road; the bonnet and front are crushed.",
            &[("lupo", false), ("c15", false)],
            DifferentVehicle,
            None,
        ),
        claim(
            15,
            "2008 Honda Civic Type R hatchback, red",
            "2026-11-12",
            "Front-end crash; the car was recovered to a tyre yard.",
            &[("typer", false)],
            Estimate,
            None,
        ),
        claim(
            16,
            "2004 Chevrolet Impala sedan, silver",
            "2026-11-13",
            "Hit in the side at a junction at night.",
            &[("impala", false)],
            Estimate,
            Some("side"),
        ),
        claim(
            17,
            "2004 Mercedes-Benz C-Class coupé, silver",
            "2026-11-14",
            "Front collision; the car is parked at the roadside awaiting recovery.",
            &[("cclass", false)],
            Estimate,
            None,
        ),
        claim(
            18,
            "2007 Chevrolet HHR, silver",
            "2026-11-15",
            "Front left corner damaged in a collision at a roundabout.",
            &[("hhr", true)],
            ReusedPhoto,
            None,
        ),
        claim(
            19,
            "2006 Mercedes-Benz E-Class sedan, black",
            "2026-11-16",
            "Front-end damage after hitting a stationary car.",
            &[("w211", true)],
            ReusedPhoto,
            None,
        ),
    ]
}

/// The insurer's illustrative price list: list prices for parts, and the labour rate.
pub struct PriceList {
    pub parts: &'static [(&'static str, f64)],
    pub other_part: f64,
    pub labour_per_hour: f64,
}

pub const PRICES: PriceList = PriceList {
    parts: &[
        ("bumper", 420.0),
        ("headlamp", 380.0),
        ("headlight", 380.0),
        ("bonnet", 520.0),
        ("hood", 520.0),
        ("wing", 290.0),
        ("fender", 290.0),
        ("door", 640.0),
        ("grille", 160.0),
        ("radiator", 340.0),
        ("windscreen", 450.0),
        ("windshield", 450.0),
        ("mirror", 180.0),
        ("wheel", 260.0),
        ("airbag", 900.0),
        ("panel", 380.0),
    ],
    other_part: 300.0,
    labour_per_hour: 62.0,
};

pub fn data_dir(root: &Path) -> PathBuf {
    root.join("data")
}

pub fn photo_path(root: &Path, name: &str) -> PathBuf {
    data_dir(root).join("photos").join(name)
}

/// Wikimedia asks automated clients to name themselves; `WIKIMEDIA_USER_AGENT` lets you add
/// your own contact, as its policy suggests.
fn user_agent() -> String {
    std::env::var("WIKIMEDIA_USER_AGENT").unwrap_or_else(|_| {
        "adk-playground-claims-desk/0.1 (https://github.com/zavora-ai/adk-playground)".into()
    })
}

/// Downloads each Commons file (checked by size and SHA-1), and writes every claim's photos:
/// scaled to 1568 pixels on the long edge, the size Claude reads without scaling again; a
/// reused photo cropped and re-compressed as a second copy would be.
pub async fn fetch(root: &Path) -> anyhow::Result<()> {
    let originals = data_dir(root).join("commons");
    std::fs::create_dir_all(&originals)?;
    std::fs::create_dir_all(data_dir(root).join("photos"))?;
    let client = reqwest::Client::builder()
        .user_agent(user_agent())
        .build()?;
    for s in &SOURCES {
        let path = originals.join(format!("{}.jpg", s.key));
        if path.exists() && hex::encode(Sha1::digest(std::fs::read(&path)?)) == s.sha1 {
            println!("  ok       {} — {}, {}", s.title, s.artist, s.licence);
            continue;
        }
        println!(
            "  fetching {} ({} bytes) — {}, {}",
            s.title, s.size, s.artist, s.licence
        );
        let mut bytes = None;
        for attempt in 1..=6u64 {
            let response = client.get(s.url).send().await?;
            if response.status().as_u16() == 429 {
                tokio::time::sleep(Duration::from_secs(30 * attempt)).await;
                continue;
            }
            bytes = Some(response.error_for_status()?.bytes().await?);
            break;
        }
        let bytes = bytes.with_context(|| {
            format!(
                "Wikimedia kept answering 429 for {}; run fetch again later",
                s.title
            )
        })?;
        let digest = hex::encode(Sha1::digest(&bytes));
        if bytes.len() as u64 != s.size || digest != s.sha1 {
            bail!(
                "{} arrived as {} bytes with sha1 {digest}; Commons lists {} bytes and {}",
                s.title,
                bytes.len(),
                s.size,
                s.sha1
            );
        }
        std::fs::File::create(&path)?.write_all(&bytes)?;
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    for claim in claims() {
        for photo in &claim.photos {
            let original = image::open(originals.join(format!("{}.jpg", photo.source)))?;
            let image = if photo.reused {
                let (w, h) = (original.width(), original.height());
                original.crop_imm(w * 4 / 100, h * 4 / 100, w * 92 / 100, h * 92 / 100)
            } else {
                original
            };
            let scaled = image.resize(1568, 1568, FilterType::Lanczos3).to_rgb8();
            let mut out = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut out,
                if photo.reused { 78 } else { 90 },
            )
            .encode_image(&scaled)?;
            std::fs::write(photo_path(root, &photo.name), out)?;
        }
    }
    println!(
        "  wrote the photos of {} claims to data/photos",
        claims().len()
    );
    Ok(())
}
