//! The measurement: near-misses in the dataset's ground truth, and how many of them the app
//! found.
//!
//! A ground-truth near-miss is a moving forklift or pallet truck (over 0.3 m/s) whose
//! footprint comes within 1 m of a person, on a floor one of the four cameras shows both of
//! them on. Samples 2 s apart or closer for the same pair are one event.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

use crate::data::{CAMERAS, Kind, TRUTH_STEP, Truth, VIDEO_FPS};

/// The footprint distance a near-miss is within.
pub const TRUTH_GAP_M: f64 = 1.0;
/// The speed a vehicle must move at for a close approach to count.
pub const MOVING_M_S: f64 = 0.3;
/// Samples of one pair this close in time are one event.
const MERGE_MS: u64 = 2_000;
/// A camera shows a pair clearly when, at some moment of the event, the vehicle's visible
/// part covers at least this many pixels of the 1920 × 1080 frame…
const CLEAR_VEHICLE_PX: f32 = 3_000.0;
/// …and the person's visible part at least this tall.
const CLEAR_PERSON_PX: f32 = 80.0;
/// A detection matches an event within this time of it…
const MATCH_MS: u64 = 3_000;
/// …and with its person this close to the event's person.
const MATCH_M: f64 = 3.0;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TruthEvent {
    pub person: u64,
    pub vehicle: u64,
    pub vehicle_kind: Kind,
    pub start_ms: u64,
    pub end_ms: u64,
    pub closest_ms: u64,
    pub closest_gap_m: f64,
    pub cameras: Vec<&'static str>,
    /// The cameras that show both clearly at some moment of the event.
    pub clear_in: Vec<&'static str>,
}

/// What the app reported: a close approach on one camera at one time, with the person's
/// floor position.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub id: String,
    pub camera: String,
    pub at_ms: u64,
    pub x: f64,
    pub y: f64,
}

/// One sample of a close pair: time, footprint gap, the cameras showing both, the vehicle's
/// kind, and the cameras showing both clearly.
type Hit = (u64, f64, u8, Kind, u8);

pub fn ms(frame: u64) -> u64 {
    frame * 1_000 / VIDEO_FPS
}

/// Every ground-truth near-miss that starts before `until_ms`.
pub fn events(truth: &Truth, until_ms: u64) -> Vec<TruthEvent> {
    let mut last: HashMap<u64, (f64, f64)> = HashMap::new();
    let mut hits: BTreeMap<(u64, u64), Vec<Hit>> = BTreeMap::new();
    let dt = TRUTH_STEP as f64 / VIDEO_FPS as f64;
    for sample in &truth.samples {
        let at = ms(sample.frame);
        if at > until_ms {
            break;
        }
        let people: Vec<_> = sample
            .objects
            .iter()
            .filter(|o| o.kind == Kind::Person)
            .collect();
        for vehicle in sample.objects.iter().filter(|o| o.kind != Kind::Person) {
            let speed = last
                .get(&vehicle.id)
                .map_or(0.0, |(x, y)| (vehicle.x - x).hypot(vehicle.y - y) / dt);
            last.insert(vehicle.id, (vehicle.x, vehicle.y));
            if speed < MOVING_M_S {
                continue;
            }
            for person in &people {
                let both = person.cameras & vehicle.cameras;
                let gap = vehicle.gap(person.x, person.y);
                if gap < TRUTH_GAP_M && both != 0 {
                    let clear = (0..CAMERAS.len()).fold(0u8, |mask, i| {
                        match (vehicle.boxes[i], person.boxes[i]) {
                            (Some(v), Some(p))
                                if (v[2] - v[0]) * (v[3] - v[1]) >= CLEAR_VEHICLE_PX
                                    && p[3] - p[1] >= CLEAR_PERSON_PX =>
                            {
                                mask | (1 << i)
                            }
                            _ => mask,
                        }
                    });
                    hits.entry((person.id, vehicle.id)).or_default().push((
                        at,
                        gap,
                        both,
                        vehicle.kind,
                        clear,
                    ));
                }
            }
        }
    }
    let mut out = Vec::new();
    for ((person, vehicle), samples) in hits {
        let mut run: Vec<Hit> = Vec::new();
        let mut flush = |run: &mut Vec<Hit>| {
            if let (Some(first), Some(last)) = (run.first(), run.last()) {
                let closest = run
                    .iter()
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .copied()
                    .unwrap_or(*first);
                let mask = run.iter().fold(0u8, |m, s| m | s.2);
                let names = |mask: u8| -> Vec<&'static str> {
                    CAMERAS
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| mask & (1 << i) != 0)
                        .map(|(_, c)| *c)
                        .collect()
                };
                out.push(TruthEvent {
                    person,
                    vehicle,
                    vehicle_kind: first.3,
                    start_ms: first.0,
                    end_ms: last.0,
                    closest_ms: closest.0,
                    closest_gap_m: (closest.1 * 100.0).round() / 100.0,
                    cameras: names(mask),
                    clear_in: names(run.iter().fold(0u8, |m, s| m | s.4)),
                });
            }
            run.clear();
        };
        for sample in samples {
            if run.last().is_some_and(|l| sample.0 - l.0 > MERGE_MS) {
                flush(&mut run);
            }
            run.push(sample);
        }
        flush(&mut run);
    }
    out.sort_by_key(|e| e.start_ms);
    out
}

/// A person's ground-truth floor position nearest `at_ms`.
fn person_at(truth: &Truth, person: u64, at_ms: u64) -> Option<(f64, f64)> {
    let index = truth
        .samples
        .partition_point(|s| ms(s.frame) < at_ms)
        .min(truth.samples.len().saturating_sub(1));
    truth
        .samples
        .get(index)?
        .objects
        .iter()
        .find(|o| o.id == person)
        .map(|o| (o.x, o.y))
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Score {
    pub truth_events: usize,
    pub reported: usize,
    pub events_found: usize,
    pub reports_on_an_event: usize,
    pub recall: f64,
    pub precision: f64,
    /// The events some camera shows clearly, and how many of them were found.
    pub clear_events: usize,
    pub clear_found: usize,
    pub clear_recall: f64,
    /// For each report, the ground-truth event it lies on, if any.
    pub matches: BTreeMap<String, Option<usize>>,
}

/// How many events the reports found, and how many reports lie on an event.
pub fn score(truth: &Truth, events: &[TruthEvent], found: &[Found]) -> Score {
    let mut matches = BTreeMap::new();
    let mut hit = vec![false; events.len()];
    for report in found {
        let on = events.iter().position(|event| {
            report.at_ms + MATCH_MS >= event.start_ms
                && report.at_ms <= event.end_ms + MATCH_MS
                && person_at(truth, event.person, report.at_ms)
                    .is_some_and(|(x, y)| (x - report.x).hypot(y - report.y) <= MATCH_M)
        });
        if let Some(index) = on {
            hit[index] = true;
        }
        matches.insert(report.id.clone(), on);
    }
    let events_found = hit.iter().filter(|h| **h).count();
    let clear: Vec<usize> = (0..events.len())
        .filter(|i| !events[*i].clear_in.is_empty())
        .collect();
    let clear_found = clear.iter().filter(|i| hit[**i]).count();
    let on_event = matches.values().filter(|m| m.is_some()).count();
    let ratio = |a: usize, b: usize| {
        if b == 0 {
            0.0
        } else {
            (a as f64 / b as f64 * 1000.0).round() / 1000.0
        }
    };
    Score {
        truth_events: events.len(),
        reported: found.len(),
        events_found,
        reports_on_an_event: on_event,
        recall: ratio(events_found, events.len()),
        precision: ratio(on_event, found.len()),
        clear_events: clear.len(),
        clear_found,
        clear_recall: ratio(clear_found, clear.len()),
        matches,
    }
}
