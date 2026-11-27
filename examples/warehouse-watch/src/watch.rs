//! The cameras: people found and tracked on this computer in every sampled frame, forklifts
//! and pallet trucks spotted by Claude once a second, both placed on the warehouse floor in
//! metres through each camera's calibration, and a close approach between a moving vehicle
//! and a person kept as an incident with its clip.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use adk_artifact::{ArtifactService, SaveRequest};
use adk_core::{ArtifactLocator, Part};
use adk_vision::stream::{FileSource, Observation, ObservationSink, RedactedFrame, StreamPipeline};
use adk_vision::{
    ByteTracker, DecodedImage, DescribeSpec, DetectQuery, FrameSourceId, Sampling, SourceImage,
    VisionCx, VisionRuntime,
};
use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::Digest as _;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::data::{self, Floor};
use crate::tape::{Mode, Tape};

/// Frames the detector looks at each second.
pub const SAMPLE_FPS: f32 = 5.0;
/// Sampled frames are 1280 × 720; the calibration is in the 1920 × 1080 frame's pixels.
pub const SAMPLE_EDGE: u32 = 1280;
const TO_CALIBRATION: f64 = 1.5;
/// How often each camera's frame goes to the vehicle spotter.
pub const SPOT_EVERY_MS: u64 = 1_000;
/// A close approach: the person's feet within this of the floor point under the vehicle's
/// box, less [`VEHICLE_REACH_M`].
pub const GAP_M: f64 = 1.5;
/// How far a vehicle's body reaches past the floor point under its box's bottom edge.
pub const VEHICLE_REACH_M: f64 = 0.9;
/// Below this a vehicle counts as standing, measured over [`SPEED_BASELINE_MS`] so the
/// jitter of a box's bottom edge does not read as motion.
pub const MOVING_M_S: f64 = 0.5;
const SPEED_BASELINE_MS: u64 = 2_000;
/// One incident per person and vehicle in this long.
const COOLDOWN_MS: u64 = 10_000;
/// The clip an incident keeps: 2 s either side, a frame every 0.5 s.
const CLIP_SIDE_MS: u64 = 2_000;
const CLIP_STEP_MS: u64 = 500;
/// A clip frame is the ring frame nearest its slot, within this; a clip is cut only once the
/// video is this far past its last slot, so a replay and a live run cut the same frames.
const CLIP_SLACK_MS: u64 = 300;
/// Frames each camera keeps in memory for clips; spot answers trail the video by at most
/// [`SPOTS_IN_FLIGHT`] seconds, so a clip's frames are still here when its answer lands.
const RING_MS: u64 = 20_000;
/// The ring keeps a frame every half second, the clip's step.
const RING_STEP_MS: u64 = 450;
/// People each camera keeps, for spot answers that land late.
const PEOPLE_MS: u64 = 60_000;
/// Spot calls each camera has open at once; answers are applied in the order of their frames.
const SPOTS_IN_FLIGHT: usize = 6;

/// The application, person and session names the app's runs and artifacts use.
pub const APP: &str = "warehouse";
pub const USER: &str = "supervisor";

/// A vehicle the spotter boxed, in sampled-frame pixels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spot {
    pub label: String,
    #[serde(rename = "box")]
    pub bbox: [f32; 4],
}

/// Finds forklifts and pallet trucks in a frame.
#[async_trait]
pub trait Spotter: Send + Sync {
    async fn spot(
        &self,
        camera: &str,
        at_ms: u64,
        frame: &DecodedImage,
    ) -> anyhow::Result<Vec<Spot>>;
}

pub const SPOT_PROMPT: &str = "This is a frame from a warehouse camera. Box every forklift \
(counterbalance, reach truck or order picker) and every pallet truck (a pallet jack, manual or \
powered) standing or moving on the floor. Label each box `forklift` or `pallet truck`. Leave \
out vehicles hidden behind racking and anything that is not a vehicle. With none in view, \
return no boxes. Keep the text to a few words.";

/// Claude through the vision runtime and its egress gate, answers recorded or replayed.
pub struct ClaudeSpotter {
    pub runtime: Arc<VisionRuntime>,
    pub tape: Arc<Tape>,
}

#[async_trait]
impl Spotter for ClaudeSpotter {
    async fn spot(
        &self,
        camera: &str,
        at_ms: u64,
        frame: &DecodedImage,
    ) -> anyhow::Result<Vec<Spot>> {
        if self.tape.mode == Mode::Replay {
            // A spot the live run could not read is missing here too, and skipped the same way.
            return self
                .tape
                .spots(camera, at_ms)
                .ok_or_else(|| anyhow::anyhow!("no recorded answer"));
        }
        let image = SourceImage::from_pixels(frame.clone())?;
        let spec = DescribeSpec::new(SPOT_PROMPT).with_regions();
        let out = self
            .runtime
            .describe(&VisionCx::new(), &[&image], &spec)
            .await?;
        let spots: Vec<Spot> = out
            .value
            .regions
            .iter()
            .filter_map(|region| {
                let label = region.label.as_deref().unwrap_or_default().to_lowercase();
                let label = if label.contains("pallet") {
                    "pallet truck"
                } else if label.contains("fork")
                    || label.contains("reach")
                    || label.contains("picker")
                {
                    "forklift"
                } else {
                    return None;
                };
                let b = region.shape.bbox()?;
                Some(Spot {
                    label: label.into(),
                    bbox: [b.x0, b.y0, b.x1, b.y1],
                })
            })
            .collect();
        self.tape.record_spots(camera, at_ms, &spots);
        Ok(spots)
    }
}

/// A person's foot point on the floor in one sampled frame.
#[derive(Clone, Debug)]
struct Seen {
    at_ms: u64,
    track: u64,
    x: f64,
    y: f64,
    bbox: [f32; 4],
}

/// A vehicle the camera follows between spots.
#[derive(Clone, Debug)]
struct Vehicle {
    id: u64,
    label: String,
    at_ms: u64,
    x: f64,
    y: f64,
    bbox: [f32; 4],
    /// Earlier positions, oldest first, for the speed baseline.
    trail: Vec<(u64, f64, f64)>,
}

/// A close approach waiting for the frames after it.
#[derive(Clone, Debug)]
struct Approach {
    at_ms: u64,
    track: u64,
    person: (f64, f64),
    person_box: [f32; 4],
    vehicle: Vehicle,
    gap_m: f64,
    speed_m_s: f64,
}

/// The frames of a clip, with their video times.
type Clip = Vec<(u64, Arc<RedactedFrame>)>;

/// One incident: a close approach with its clip, measured on the floor.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Incident {
    pub id: String,
    pub camera: String,
    pub at_ms: u64,
    pub person_track: u64,
    pub person_floor_m: [f64; 2],
    pub person_box: [f32; 4],
    pub vehicle: String,
    pub vehicle_id: u64,
    pub vehicle_floor_m: [f64; 2],
    pub vehicle_box: [f32; 4],
    pub gap_m: f64,
    pub vehicle_speed_m_s: f64,
    /// The clip's frames, artifacts in the incident's session, oldest first.
    pub frames: Vec<String>,
    pub times_ms: Vec<u64>,
    /// The frame of the closest approach.
    pub key_frame: String,
    /// The key frame's PNG digest, `sha256:…`.
    pub key_digest: String,
    #[serde(skip)]
    pub key_locator: Option<ArtifactLocator>,
    pub width: u32,
    pub height: u32,
}

/// Every incident the cameras raised, and who to tell about a new one.
pub struct Incidents {
    pub book: Mutex<BTreeMap<String, Incident>>,
    artifacts: Arc<dyn ArtifactService>,
    notify: mpsc::UnboundedSender<Incident>,
}

impl Incidents {
    pub fn new(
        artifacts: Arc<dyn ArtifactService>,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<Incident>) {
        let (notify, rx) = mpsc::unbounded_channel();
        (
            Arc::new(Self {
                book: Mutex::new(BTreeMap::new()),
                artifacts,
                notify,
            }),
            rx,
        )
    }

    pub fn get(&self, id: &str) -> Option<Incident> {
        self.book
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    pub fn all(&self) -> Vec<Incident> {
        self.book
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect()
    }

    async fn save(
        &self,
        session: &str,
        name: &str,
        png: Vec<u8>,
    ) -> anyhow::Result<ArtifactLocator> {
        let saved = self
            .artifacts
            .save(SaveRequest {
                app_name: APP.into(),
                user_id: USER.into(),
                session_id: session.into(),
                file_name: name.into(),
                part: Part::InlineData {
                    mime_type: "image/png".into(),
                    data: png,
                    uri: None,
                    annotations: None,
                },
                version: None,
            })
            .await?;
        Ok(ArtifactLocator::new(
            APP,
            USER,
            session,
            name,
            saved.version,
        ))
    }
}

/// What one camera holds between frames.
#[derive(Default)]
struct State {
    ring: VecDeque<(u64, Arc<RedactedFrame>)>,
    people: VecDeque<Seen>,
    next_spot_ms: u64,
    vehicles: Vec<Vehicle>,
    next_vehicle: u64,
    pending: Vec<Approach>,
    fired: HashMap<(u64, u64), u64>,
    spots: u64,
    /// The newest frame and the people in it, for the camera page.
    latest: Option<(u64, Arc<RedactedFrame>, Vec<[f32; 4]>)>,
}

/// One camera's sink: the people it tracks, its spot queue and its close approaches.
pub struct Camera {
    pub name: String,
    pub short: String,
    floor: Floor,
    state: Mutex<State>,
    spot_queue: mpsc::Sender<(u64, Arc<RedactedFrame>)>,
    incidents: Arc<Incidents>,
}

impl Camera {
    /// A camera over `name`'s calibration, its spot queue served by `spotter`.
    pub fn new(
        name: &str,
        floor: Floor,
        spotter: Arc<dyn Spotter>,
        incidents: Arc<Incidents>,
    ) -> Arc<Self> {
        let (tx, mut rx) = mpsc::channel::<(u64, Arc<RedactedFrame>)>(1);
        let camera = Arc::new(Self {
            name: name.into(),
            short: format!("c{}", &name[name.len() - 2..]),
            floor,
            state: Mutex::new(State::default()),
            spot_queue: tx,
            incidents,
        });
        let worker = Arc::clone(&camera);
        tokio::spawn(async move {
            let frames = futures::stream::poll_fn(move |cx| rx.poll_recv(cx));
            let name = worker.name.clone();
            let mut answers = frames
                .map(|(at_ms, frame): (u64, Arc<RedactedFrame>)| {
                    let spotter = Arc::clone(&spotter);
                    let name = name.clone();
                    async move { (at_ms, spotter.spot(&name, at_ms, frame.pixels()).await) }
                })
                .buffered(SPOTS_IN_FLIGHT);
            while let Some((at_ms, answer)) = answers.next().await {
                match answer {
                    Ok(spots) => worker.spotted(at_ms, spots),
                    Err(err) => {
                        tracing::warn!(camera = %worker.name, at = %clock(at_ms), error = %err, "spot skipped")
                    }
                }
            }
        });
        camera
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn spots(&self) -> u64 {
        self.state().spots
    }

    /// The newest frame at half size as a PNG, people outlined in green and the vehicles of
    /// the last spot in orange, with its video time.
    pub fn picture(&self) -> Option<(u64, Vec<u8>)> {
        let (at_ms, frame, people, vehicles) = {
            let state = self.state();
            let (at, frame, people) = state.latest.clone()?;
            let vehicles: Vec<[f32; 4]> = state.vehicles.iter().map(|v| v.bbox).collect();
            (at, frame, people, vehicles)
        };
        let pixels = frame.pixels();
        let (w, h) = pixels.size();
        let (hw, hh) = (w / 2, h / 2);
        let source = pixels.rgba();
        let mut out = vec![0u8; (hw * hh * 4) as usize];
        for y in 0..hh {
            for x in 0..hw {
                let s = (((y * 2) * w + x * 2) * 4) as usize;
                let d = ((y * hw + x) * 4) as usize;
                out[d..d + 4].copy_from_slice(&source[s..s + 4]);
            }
        }
        let mut draw = |b: &[f32; 4], rgb: [u8; 3]| {
            let [x0, y0, x1, y1] = b.map(|v| (v / 2.0).max(0.0) as u32);
            let (x1, y1) = (x1.min(hw - 1), y1.min(hh - 1));
            for t in 0..2 {
                for x in x0..=x1 {
                    for y in [y0.saturating_add(t).min(hh - 1), y1.saturating_sub(t)] {
                        let d = ((y * hw + x) * 4) as usize;
                        out[d..d + 3].copy_from_slice(&rgb);
                    }
                }
                for y in y0..=y1 {
                    for x in [x0.saturating_add(t).min(hw - 1), x1.saturating_sub(t)] {
                        let d = ((y * hw + x) * 4) as usize;
                        out[d..d + 3].copy_from_slice(&rgb);
                    }
                }
            }
        };
        for b in &people {
            draw(b, [34, 197, 94]);
        }
        for b in &vehicles {
            draw(b, [255, 106, 31]);
        }
        let png = DecodedImage::from_rgba(hw, hh, out)
            .ok()?
            .encode_png()
            .ok()?;
        Some((at_ms, png))
    }

    /// The floor point under a sampled-frame box's bottom centre.
    fn foot(&self, b: &[f32; 4]) -> (f64, f64) {
        let u = f64::from(b[0] + b[2]) / 2.0 * TO_CALIBRATION;
        let v = f64::from(b[3]) * TO_CALIBRATION;
        self.floor.floor(u, v)
    }

    /// A spot's vehicles placed on the floor, followed from the last spot, and checked against
    /// every person seen since.
    fn spotted(&self, at_ms: u64, spots: Vec<Spot>) {
        let mut state = self.state();
        state.spots += 1;
        let previous = std::mem::take(&mut state.vehicles);
        let mut taken = vec![false; previous.len()];
        let mut current = Vec::new();
        for spot in spots {
            let (x, y) = self.foot(&spot.bbox);
            // A forklift covers well under 3 m a second.
            let since = previous
                .first()
                .map_or(0, |v| at_ms.saturating_sub(v.at_ms));
            let reach = 3.0 * (since as f64 / 1_000.0).max(1.0);
            let nearest = previous
                .iter()
                .enumerate()
                .filter(|(i, v)| !taken[*i] && (v.x - x).hypot(v.y - y) <= reach)
                .min_by(|a, b| {
                    (a.1.x - x)
                        .hypot(a.1.y - y)
                        .total_cmp(&(b.1.x - x).hypot(b.1.y - y))
                });
            let id = match nearest {
                Some((index, vehicle)) => {
                    taken[index] = true;
                    let before = vehicle.clone();
                    let mut trail = before.trail.clone();
                    trail.push((before.at_ms, before.x, before.y));
                    trail.retain(|(t, _, _)| at_ms - t <= SPEED_BASELINE_MS + 500);
                    let now = Vehicle {
                        id: before.id,
                        label: spot.label.clone(),
                        at_ms,
                        x,
                        y,
                        bbox: spot.bbox,
                        trail,
                    };
                    // Speed over the longest baseline the trail holds, up to two seconds; after
                    // a gap of missed spots the trail is empty and the speed is not known yet.
                    let Some(&(t0, x0, y0)) = now.trail.first() else {
                        current.push(now);
                        continue;
                    };
                    let speed = (x - x0).hypot(y - y0) / ((at_ms - t0) as f64 / 1_000.0).max(0.2);
                    if speed >= MOVING_M_S {
                        self.check(&mut state, &before, &now, speed);
                    }
                    current.push(now);
                    continue;
                }
                None => {
                    state.next_vehicle += 1;
                    state.next_vehicle
                }
            };
            current.push(Vehicle {
                id,
                label: spot.label,
                at_ms,
                x,
                y,
                bbox: spot.bbox,
                trail: Vec::new(),
            });
        }
        state.vehicles = current;
    }

    /// Every person seen between two spots of a moving vehicle, against the vehicle's position
    /// interpolated to the person's frame.
    fn check(&self, state: &mut State, before: &Vehicle, now: &Vehicle, speed: f64) {
        let span = (now.at_ms - before.at_ms).max(1) as f64;
        let mut closest: HashMap<u64, (f64, Seen)> = HashMap::new();
        for seen in state
            .people
            .iter()
            .filter(|s| s.at_ms > before.at_ms && s.at_ms <= now.at_ms)
        {
            let t = (seen.at_ms - before.at_ms) as f64 / span;
            let (vx, vy) = (
                before.x + (now.x - before.x) * t,
                before.y + (now.y - before.y) * t,
            );
            let gap = ((seen.x - vx).hypot(seen.y - vy) - VEHICLE_REACH_M).max(0.0);
            if gap < GAP_M && closest.get(&seen.track).is_none_or(|(g, _)| gap < *g) {
                closest.insert(seen.track, (gap, seen.clone()));
            }
        }
        for (track, (gap, seen)) in closest {
            let key = (track, now.id);
            if state
                .fired
                .get(&key)
                .is_some_and(|last| seen.at_ms < last + COOLDOWN_MS)
            {
                continue;
            }
            state.fired.insert(key, seen.at_ms);
            state.pending.push(Approach {
                at_ms: seen.at_ms,
                track,
                person: (seen.x, seen.y),
                person_box: seen.bbox,
                vehicle: now.clone(),
                gap_m: (gap * 10.0).round() / 10.0,
                speed_m_s: (speed * 10.0).round() / 10.0,
            });
        }
    }

    /// The approaches whose clip is complete, taken out of the pending list with their frames.
    fn ready(&self, now_ms: u64) -> Vec<(Approach, Clip)> {
        let mut state = self.state();
        let (ready, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut state.pending)
            .into_iter()
            .partition(|a| a.at_ms + CLIP_SIDE_MS + CLIP_SLACK_MS <= now_ms);
        state.pending = waiting;
        ready
            .into_iter()
            .map(|approach| {
                let start = approach.at_ms.saturating_sub(CLIP_SIDE_MS);
                let mut clip: Clip = Vec::new();
                for k in 0..=(2 * CLIP_SIDE_MS / CLIP_STEP_MS) {
                    let target = start + k * CLIP_STEP_MS;
                    let nearest = state
                        .ring
                        .iter()
                        .min_by_key(|(at, _)| at.abs_diff(target))
                        .filter(|(at, _)| at.abs_diff(target) <= CLIP_SLACK_MS);
                    // One ring frame can be nearest to two targets; it goes in once.
                    if let Some(frame) = nearest
                        && clip.last().is_none_or(|(last, _)| *last != frame.0)
                    {
                        clip.push(frame.clone());
                    }
                }
                (approach, clip)
            })
            .collect()
    }

    async fn keep(
        &self,
        approach: Approach,
        clip: Vec<(u64, Arc<RedactedFrame>)>,
    ) -> anyhow::Result<()> {
        // Two people can come close to vehicles in the same frame; each incident keeps its
        // own id, a millisecond later.
        let id = {
            let book = self
                .incidents
                .book
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let mut ms = approach.at_ms;
            while book.contains_key(&format!("{}-{ms:06}", self.short)) {
                ms += 1;
            }
            format!("{}-{ms:06}", self.short)
        };
        let first = clip.first().map_or(approach.at_ms, |(at, _)| *at);
        let key_index = clip
            .iter()
            .enumerate()
            .min_by_key(|(_, (at, _))| at.abs_diff(approach.at_ms))
            .map_or(0, |(i, _)| i);
        let mut frames = Vec::new();
        let mut times = Vec::new();
        let mut key_locator = None;
        let mut key_digest = String::new();
        let (mut width, mut height) = (0, 0);
        for (k, (at, frame)) in clip.iter().enumerate() {
            let name = format!("{id}-{k}.png");
            let pixels = frame.pixels();
            (width, height) = pixels.size();
            let png = pixels.encode_png()?;
            let digest = format!("sha256:{}", hex::encode(sha2::Sha256::digest(&png)));
            let locator = self.incidents.save(&id, &name, png).await?;
            if k == key_index {
                key_locator = Some(locator);
                key_digest = digest;
            }
            frames.push(name);
            times.push(at - first);
        }
        let (vx, vy) = (approach.vehicle.x, approach.vehicle.y);
        let incident = Incident {
            id: id.clone(),
            camera: self.name.clone(),
            at_ms: approach.at_ms,
            person_track: approach.track,
            person_floor_m: [round2(approach.person.0), round2(approach.person.1)],
            person_box: approach.person_box,
            vehicle: approach.vehicle.label.clone(),
            vehicle_id: approach.vehicle.id,
            vehicle_floor_m: [round2(vx), round2(vy)],
            vehicle_box: approach.vehicle.bbox,
            gap_m: approach.gap_m,
            vehicle_speed_m_s: approach.speed_m_s,
            key_frame: frames.get(key_index).cloned().unwrap_or_default(),
            key_digest,
            frames,
            times_ms: times,
            key_locator,
            width,
            height,
        };
        println!(
            "[watch] {id} {}: person {} and a {} {:.1} m apart at {} ({:.1} m/s)",
            self.name,
            incident.person_track,
            incident.vehicle,
            incident.gap_m,
            clock(incident.at_ms),
            incident.vehicle_speed_m_s
        );
        self.incidents
            .book
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, incident.clone());
        let _ = self.incidents.notify.send(incident);
        Ok(())
    }
}

#[async_trait]
impl ObservationSink for Camera {
    async fn observe(&self, _cx: &VisionCx, observation: Observation) -> adk_vision::Result<()> {
        let at_ms = observation.frame.pts_ms;
        {
            let mut state = self.state();
            if state
                .ring
                .back()
                .is_none_or(|(at, _)| at_ms >= at + RING_STEP_MS)
            {
                state
                    .ring
                    .push_back((at_ms, Arc::clone(&observation.image)));
            }
            while state
                .ring
                .front()
                .is_some_and(|(at, _)| at + RING_MS < at_ms)
            {
                state.ring.pop_front();
            }
            let mut boxes = Vec::new();
            for region in &observation.regions {
                let (Some(track), Some(b)) = (region.track, region.shape.bbox()) else {
                    continue;
                };
                if region.label.as_deref() != Some("person") {
                    continue;
                }
                let bbox = [b.x0, b.y0, b.x1, b.y1];
                let (x, y) = self.foot(&bbox);
                boxes.push(bbox);
                state.people.push_back(Seen {
                    at_ms,
                    track: track.0,
                    x,
                    y,
                    bbox,
                });
            }
            state.latest = Some((at_ms, Arc::clone(&observation.image), boxes));
            while state
                .people
                .front()
                .is_some_and(|s| s.at_ms + PEOPLE_MS < at_ms)
            {
                state.people.pop_front();
            }
        }
        let spot = {
            let mut state = self.state();
            (at_ms >= state.next_spot_ms).then(|| {
                state.next_spot_ms = (at_ms / SPOT_EVERY_MS + 1) * SPOT_EVERY_MS;
                at_ms - at_ms % SPOT_EVERY_MS
            })
        };
        // Waiting for room holds the video back rather than skipping a spot, so every run of
        // the same video spots the same frames.
        if let Some(spot_ms) = spot {
            let _ = self
                .spot_queue
                .send((spot_ms, Arc::clone(&observation.image)))
                .await;
        }
        for (approach, clip) in self.ready(at_ms) {
            if let Err(err) = self.keep(approach, clip).await {
                eprintln!("[watch] keeping an incident failed: {err}");
            }
        }
        Ok(())
    }
}

/// Plays one camera's video through the pipeline: RF-DETR finds people, ByteTrack follows
/// them, and the camera's sink does the rest.
///
/// The source is always read losslessly: every sampled frame reaches the sink, so every run of
/// the same video sees the same approaches and finds its recorded answers. `paced_from` plays
/// the video at its own speed from that instant: the sink waits for each frame's video time,
/// so a busy machine plays late but never skips a frame. Without it the video plays as fast as
/// the machine can.
pub async fn play(
    root: &Path,
    camera: Arc<Camera>,
    detector: Arc<VisionRuntime>,
    paced_from: Option<Instant>,
    seconds: Option<u64>,
) -> anyhow::Result<adk_vision::stream::StreamSummary> {
    let source = Arc::new(FileSource::new(
        FrameSourceId(camera.name.clone()),
        data::video(root, &camera.name),
    ));
    let sink: Arc<dyn ObservationSink> = Arc::clone(&camera) as Arc<dyn ObservationSink>;
    let stop = CancellationToken::new();
    let pipeline = StreamPipeline::builder(source)
        .sampling(Sampling {
            fps: SAMPLE_FPS,
            max_long_edge: Some(SAMPLE_EDGE),
        })
        .detector(detector, DetectQuery::Classes(vec!["person".into()]))
        .tracker(Box::new(ByteTracker::default()))
        .sink(Arc::new(Until {
            inner: sink,
            until_ms: seconds.map(|s| s * 1_000),
            paced_from,
            stop: stop.clone(),
        }))
        .build()?;
    match pipeline.run(&VisionCx::new().with_cancellation(stop)).await {
        Ok(summary) => Ok(summary),
        Err(adk_vision::VisionError::Cancelled) => Ok(Default::default()),
        Err(err) => Err(err.into()),
    }
}

/// Stops the stream after `until_ms` of video, and holds each frame back until its video time
/// when the video plays at its own speed.
struct Until {
    inner: Arc<dyn ObservationSink>,
    until_ms: Option<u64>,
    paced_from: Option<Instant>,
    stop: CancellationToken,
}

#[async_trait]
impl ObservationSink for Until {
    async fn observe(&self, cx: &VisionCx, observation: Observation) -> adk_vision::Result<()> {
        if self
            .until_ms
            .is_some_and(|until| observation.frame.pts_ms > until)
        {
            self.stop.cancel();
            return Ok(());
        }
        // Waiting here holds the lossless source back; a frame already late goes on at once.
        if let Some(start) = self.paced_from {
            let due = start + Duration::from_millis(observation.frame.pts_ms);
            tokio::time::sleep_until(due.into()).await;
        }
        self.inner.observe(cx, observation).await
    }
}

pub fn clock(ms: u64) -> String {
    format!("{}:{:04.1}", ms / 60_000, (ms % 60_000) as f64 / 1_000.0)
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
