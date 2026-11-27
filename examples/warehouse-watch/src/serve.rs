//! The Warehouse Watch page, the whole product for the shift supervisor: the four cameras, each
//! close approach with the frame and the agent's review, the near-misses the agent raises
//! decided on the incident itself, and questions about it. The shift starts when the
//! supervisor presses **Start the shift**; the web home at `/ui/` stays as the developer view.

use std::collections::HashMap;
use std::future::IntoFuture;
use std::path::Path as FilePath;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adk_artifact::{ArtifactService, LoadRequest};
use adk_core::{
    ApprovalRequest, ApprovalState, RequestContext, RunRecord, RunState, SingleAgentLoader,
};
use adk_server::auth_bridge::{RequestContextError, RequestContextExtractor};
use adk_server::{ServerBuilder, ServerConfig};
use adk_surface::{CardHint, CardHints, LocalSource, Place, SurfaceEngine};
use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::agent::Desk;
use crate::tape::{Mode, Tape};
use crate::watch::{APP, Camera, Incident, Incidents, USER, clock};
use crate::{App, Common, data};

/// Admits every request as the supervisor, presence confirmed: the server listens on loopback.
struct Supervisor;

#[async_trait]
impl RequestContextExtractor for Supervisor {
    async fn extract(
        &self,
        _parts: &axum::http::request::Parts,
    ) -> Result<RequestContext, RequestContextError> {
        Ok(RequestContext {
            user_id: USER.into(),
            scopes: vec!["owner".into(), "operator".into()],
            metadata: [(
                adk_surface::web::PRESENCE_METADATA_KEY.to_string(),
                "confirmed".to_string(),
            )]
            .into_iter()
            .collect(),
        })
    }
}

/// The heading an incident card shows.
struct IncidentCards;

#[async_trait]
impl CardHints for IncidentCards {
    async fn hint(&self, request: &adk_core::ApprovalRequest) -> Option<CardHint> {
        (request.tool_name == "raise_incident").then(|| {
            let title = request.args_redacted["title"]
                .as_str()
                .unwrap_or("Near-miss to sign off");
            CardHint::new().with_heading(title.to_string())
        })
    }
}

/// Where the shift is: waiting for the supervisor, playing since an instant, or over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Waiting,
    Running(Instant),
    Finished,
}

#[derive(Clone)]
struct Page {
    cameras: Vec<Arc<Camera>>,
    incidents: Arc<Incidents>,
    desk: Arc<Desk>,
    tape: Arc<Tape>,
    daily_usd: f64,
    total_ms: u64,
    shift: Arc<watch::Sender<Phase>>,
    /// Each camera's first frame, shown before the shift starts.
    posters: Arc<HashMap<String, Vec<u8>>>,
}

pub async fn serve(common: Common, port: u16) -> anyhow::Result<()> {
    let App {
        tape,
        detector,
        cameras,
        incidents,
        desk,
        incoming,
    } = App::build(&common).await?;
    // A question or a decision's closing turn the recording does not hold gets a reply that
    // says so, never a silent failure.
    tape.answer_unrecorded_in_words();
    let desk = Arc::new(desk);
    desk.kernel.start().await?;
    crate::dispatch(Arc::clone(&desk), incoming);

    let local = LocalSource::builder(APP)
        .sessions(desk.sessions.clone())
        .approvals(desk.approvals.clone())
        .harness(desk.harness.clone())
        .kernel(desk.kernel.clone(), APP)
        .artifacts(desk.artifacts.clone() as Arc<dyn ArtifactService>)
        .runs(desk.runs.clone())
        .build()?;
    let engine = SurfaceEngine::builder(Arc::new(local))
        .places([Place::web()])
        .agent_name(APP, "Warehouse Watch")
        .card_hints(Arc::new(IncidentCards))
        .build();
    let config = ServerConfig::new(
        Arc::new(SingleAgentLoader::new(desk.agent.clone())),
        desk.sessions.clone(),
    )
    .with_request_context(Arc::new(Supervisor));

    let root = crate::root();
    let posters = futures::future::join_all(cameras.iter().map(|camera| {
        let root = root.clone();
        let name = camera.name.clone();
        async move { poster(&root, &name).await.map(|png| (name, png)) }
    }))
    .await
    .into_iter()
    .flatten()
    .collect();
    let (shift, _) = watch::channel(Phase::Waiting);
    let shift = Arc::new(shift);
    let page = Page {
        cameras: cameras.clone(),
        incidents: Arc::clone(&incidents),
        desk: Arc::clone(&desk),
        tape: Arc::clone(&tape),
        daily_usd: common.daily_usd,
        total_ms: common.seconds.map_or(300_000, |s| s * 1_000),
        shift: Arc::clone(&shift),
        posters: Arc::new(posters),
    };
    let warehouse = axum::Router::new()
        .route("/warehouse", get(|| async { Html(PAGE) }))
        .route("/warehouse/state", get(state))
        .route("/warehouse/start", post(start))
        .route("/warehouse/camera/{name}", get(picture))
        .route("/warehouse/incidents/{id}", get(incident))
        .route("/warehouse/incidents/{id}/frames/{index}", get(frame))
        .with_state(page);
    let app = ServerBuilder::new(config)
        .add_api_routes(adk_surface::web::router(engine))
        .add_root_routes(adk_surface::web::home().merge(warehouse))
        .build();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("Warehouse Watch on http://127.0.0.1:{port}/warehouse: press Start the shift there");
    println!("the developer view on http://127.0.0.1:{port}/ui/");
    let stopping = Arc::new(tokio::sync::Notify::new());
    let server = tokio::spawn(
        axum::serve(listener, app)
            .with_graceful_shutdown({
                let stopping = Arc::clone(&stopping);
                async move { stopping.notified().await }
            })
            .into_future(),
    );

    // The shift plays once, at the video's own speed, from the moment the supervisor starts
    // it; the page stays up after it.
    let seconds = common.seconds;
    let plays = tokio::spawn(async move {
        let mut phase = shift.subscribe();
        let started = loop {
            if let Phase::Running(at) = *phase.borrow_and_update() {
                break at;
            }
            if phase.changed().await.is_err() {
                return;
            }
        };
        println!("[shift] started from the page");
        let plays = cameras.iter().map(|camera| {
            crate::watch::play(
                &root,
                Arc::clone(camera),
                Arc::clone(&detector),
                Some(started),
                seconds,
            )
        });
        for result in futures::future::join_all(plays).await {
            if let Err(err) = result {
                eprintln!("[camera] stopped: {err}");
            }
        }
        shift.send_replace(Phase::Finished);
        println!("[shift] the video has ended; the page stays open until Ctrl-C");
    });
    tokio::signal::ctrl_c().await?;
    println!();
    println!("stopping: the cameras, then the server and the kernel…");
    plays.abort();
    stopping.notify_one();
    let _ = tokio::time::timeout(Duration::from_secs(3), server).await;
    desk.kernel.stop().await?;
    tape.save()?;
    Ok(())
}

/// A camera's first frame at half size, through ffmpeg, for the page before the shift starts.
async fn poster(root: &FilePath, camera: &str) -> Option<Vec<u8>> {
    let binary = std::env::var_os("ADK_VISION_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    let out = tokio::process::Command::new(binary)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(data::video(root, camera))
        .args(["-frames:v", "1", "-vf", "scale=640:-2"])
        .args(["-f", "image2pipe", "-c:v", "png", "-"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    (out.status.success() && !out.stdout.is_empty()).then_some(out.stdout)
}

/// `Camera_0015` as the page names it: `Camera 15`.
fn label(camera: &str) -> String {
    format!(
        "Camera {}",
        camera.trim_start_matches("Camera_").trim_start_matches('0')
    )
}

fn png(bytes: Vec<u8>, cache: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, cache),
        ],
        bytes,
    )
        .into_response()
}

async fn start(State(page): State<Page>) -> StatusCode {
    page.shift.send_if_modified(|phase| {
        let waiting = *phase == Phase::Waiting;
        if waiting {
            *phase = Phase::Running(Instant::now());
        }
        waiting
    });
    StatusCode::NO_CONTENT
}

/// The camera's newest frame, or its first frame before the shift reaches it.
async fn picture(State(page): State<Page>, Path(name): Path<String>) -> Response {
    let name = name.trim_end_matches(".png");
    let Some(camera) = page.cameras.iter().find(|c| c.name == name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let live = tokio::task::spawn_blocking({
        let camera = Arc::clone(camera);
        move || camera.picture()
    })
    .await
    .ok()
    .flatten();
    match (live, page.posters.get(name)) {
        (Some((_, bytes)), _) => png(bytes, "no-store"),
        (None, Some(poster)) => png(poster.clone(), "no-store"),
        (None, None) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// An incident's status as the list and the panel show it, and its kind for the colour.
fn status(
    decided: Option<&str>,
    card: Option<&ApprovalRequest>,
    turn: Option<&RunRecord>,
) -> (&'static str, &'static str) {
    match (decided, card.map(|c| c.state)) {
        (Some("raise_incident"), Some(ApprovalState::Pending)) => {
            ("waiting", "Waiting for your sign-off")
        }
        (Some("raise_incident"), Some(ApprovalState::Approved)) => ("signed", "Signed off"),
        (Some("raise_incident"), Some(ApprovalState::Rejected)) => ("declined", "Declined"),
        (
            Some("raise_incident"),
            Some(ApprovalState::Responded | ApprovalState::Expired | ApprovalState::Cancelled),
        ) => ("closed", "Closed without sign-off"),
        (Some("raise_incident"), None) => ("reviewing", "Reviewing"),
        (Some(_), _) => ("logged", "Logged, not a near-miss"),
        (None, _) => match turn.map(|t| t.state) {
            Some(RunState::Failed) => ("failed", "Review failed"),
            Some(RunState::Completed | RunState::Cancelled) => ("done", "Reviewed, no decision"),
            // Queued, running or waiting; `RunState` is non-exhaustive.
            _ => ("reviewing", "Reviewing"),
        },
    }
}

fn run_state(state: RunState) -> &'static str {
    match state {
        RunState::Queued => "queued",
        RunState::Running => "running",
        RunState::Waiting => "waiting",
        RunState::Completed => "completed",
        RunState::Failed => "failed",
        RunState::Cancelled => "cancelled",
        // `RunState` is non-exhaustive; a state added later reads as still going.
        _ => "running",
    }
}

async fn state(State(page): State<Page>) -> impl IntoResponse {
    let cards = page.desk.cards().await;
    let turns = page.desk.turns().await;
    let mut rows = Vec::new();
    let (mut raised, mut logged, mut waiting) = (0, 0, 0);
    for incident in page.incidents.all().into_iter().rev() {
        let decided = page.desk.decision(&incident.id).await;
        let (kind, text) = status(
            decided.as_deref(),
            cards.get(&incident.id),
            turns.get(&incident.id),
        );
        match decided.as_deref() {
            Some("raise_incident") => raised += 1,
            Some(_) => logged += 1,
            None => {}
        }
        if kind == "waiting" {
            waiting += 1;
        }
        rows.push(json!({
            "id": incident.id,
            "camera": label(&incident.camera),
            "at": clock(incident.at_ms),
            "gapM": incident.gap_m,
            "vehicle": incident.vehicle,
            "speed": incident.vehicle_speed_m_s,
            "status": kind,
            "statusText": text,
        }));
    }
    let phase = *page.shift.borrow();
    let cameras: Vec<_> = page
        .cameras
        .iter()
        .map(|c| {
            json!({
                "name": c.name,
                "label": label(&c.name),
                "atMs": c.at_ms(),
                "poster": page.posters.contains_key(&c.name),
            })
        })
        .collect();
    let at_ms = match phase {
        Phase::Waiting => 0,
        Phase::Running(_) => page
            .cameras
            .iter()
            .map(|c| c.at_ms().unwrap_or(0))
            .min()
            .unwrap_or(0)
            .min(page.total_ms),
        Phase::Finished => page.total_ms,
    };
    // A fold from +0.0: an empty float sum is -0.0, and a live run starts with no spend.
    let spend = page.tape.spend().values().fold(0.0, |sum, s| sum + s.usd);
    axum::Json(json!({
        "mode": if page.tape.mode == Mode::Replay { "replay" } else { "live" },
        "shift": {
            "phase": match phase {
                Phase::Waiting => "waiting",
                Phase::Running(_) => "running",
                Phase::Finished => "finished",
            },
            "atMs": at_ms,
            "totalMs": page.total_ms,
        },
        "cameras": cameras,
        "incidents": rows,
        "raised": raised,
        "logged": logged,
        "waiting": waiting,
        "spendUsd": (spend * 10_000.0).round() / 10_000.0,
        "capUsd": page.daily_usd,
        "source": format!("{}@{} {}", data::REPO, &data::REVISION[..10], data::SCENE),
    }))
}

/// One incident for the panel: the measurement, the clip, the agent's review, the card's
/// answer and the state of the latest turn.
async fn incident(State(page): State<Page>, Path(id): Path<String>) -> Response {
    let Some(incident) = page.incidents.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let review = page.desk.review(&id).await;
    let cards = page.desk.cards().await;
    let card = cards.get(&id);
    let turns = page.desk.turns().await;
    let turn = turns.get(&id);
    let (kind, text) = status(review.as_ref().map(|(t, _)| t.as_str()), card, turn);
    axum::Json(json!({
        "id": incident.id,
        "camera": label(&incident.camera),
        "at": clock(incident.at_ms),
        "gapM": incident.gap_m,
        "vehicle": incident.vehicle,
        "speed": incident.vehicle_speed_m_s,
        "personBox": incident.person_box,
        "vehicleBox": incident.vehicle_box,
        "width": incident.width,
        "height": incident.height,
        "frames": clip(&incident),
        "keyIndex": incident.frames.iter().position(|f| *f == incident.key_frame),
        "task": crate::agent::task(&incident),
        "status": kind,
        "statusText": text,
        "review": review.map(|(tool, args)| json!({ "tool": tool, "args": args })),
        "card": card.map(answered),
        "turn": turn.map(|run| json!({
            "state": run_state(run.state),
            "error": run.error.as_deref().map(|e| e.chars().take(400).collect::<String>()),
        })),
    }))
    .into_response()
}

/// The clip's frames with their times from the closest approach.
fn clip(incident: &Incident) -> Vec<Value> {
    let key = incident
        .frames
        .iter()
        .position(|f| *f == incident.key_frame)
        .and_then(|k| incident.times_ms.get(k).copied())
        .unwrap_or(0);
    incident
        .times_ms
        .iter()
        .map(|&t| {
            let offset = (t as f64 - key as f64) / 1_000.0;
            json!({
                "at": if offset.abs() < 0.05 {
                    "closest".to_string()
                } else {
                    format!("{offset:+.1} s")
                },
            })
        })
        .collect()
}

/// How the supervisor answered a card, for the collapsed state.
fn answered(card: &ApprovalRequest) -> Value {
    let decision = card.decisions.last();
    json!({
        "state": match card.state {
            ApprovalState::Pending => "waiting",
            ApprovalState::Approved => "approved",
            ApprovalState::Rejected => "declined",
            ApprovalState::Responded | ApprovalState::Expired | ApprovalState::Cancelled => "closed",
        },
        "atText": card
            .resolved_at
            .or(decision.map(|d| d.decided_at))
            .map(|at| at.with_timezone(&chrono::Local).format("%H:%M").to_string()),
        "reason": decision.and_then(|d| d.reason.clone()),
    })
}

/// One frame of an incident's clip, from the artifacts in its session.
async fn frame(State(page): State<Page>, Path((id, index)): Path<(String, String)>) -> Response {
    let Some(incident) = page.incidents.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let index = index.trim_end_matches(".png");
    let Some(name) = index
        .parse::<usize>()
        .ok()
        .and_then(|k| incident.frames.get(k))
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let loaded = page
        .desk
        .artifacts
        .load(LoadRequest {
            app_name: APP.into(),
            user_id: USER.into(),
            session_id: id.clone(),
            file_name: name.clone(),
            version: None,
        })
        .await;
    match loaded.map(|l| l.part) {
        Ok(adk_core::Part::InlineData { data, .. }) => png(data, "private, max-age=3600"),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

const PAGE: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="icon" href="data:,">
<title>Warehouse Watch</title>
<style>
:root { --bg:#f6f5f3; --card:#fff; --border:#e5e3de; --text:#18181b; --text2:#45454d; --label:#6b6b74;
  --btn:#c2410c; --btnText:#fff; --btn2:#f1efeb; --alert:#b42318; --alertBg:#fdecea; --ok:#1e6b45; --okBg:#e7f4ec;
  --warn:#8a4b00; --warnBg:#fdf3e1; --mute:#ecebe7; --sel:#fff4ec; --focus:#c2410c; }
@media (prefers-color-scheme: dark) { :root { --bg:#0e0f11; --card:#16171a; --border:#2a2b30; --text:#ededef; --text2:#c2c2ca;
  --label:#8e8e97; --btn:#ff7a33; --btnText:#0e0f11; --btn2:#23242a; --alert:#ff8a80; --alertBg:#3a1d1b; --ok:#7ad3a0; --okBg:#17301f;
  --warn:#f5c26b; --warnBg:#3a2c12; --mute:#23242a; --sel:#2a1d14; --focus:#ff7a33; } }
* { box-sizing:border-box; }
[hidden] { display:none !important; }
body { margin:0; font:14px/1.45 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif; background:var(--bg); color:var(--text); }
button, textarea { font:inherit; color:inherit; }
:focus-visible { outline:2px solid var(--focus); outline-offset:2px; }
header { display:flex; flex-wrap:wrap; align-items:center; gap:10px 16px; padding:12px 24px; background:var(--card); border-bottom:1px solid var(--border); }
.brand { display:flex; align-items:center; gap:10px; font-weight:700; font-size:16px; }
.brand i { width:26px; height:26px; border-radius:7px; background:#f74c00; display:inline-block; }
.badge { font-size:12px; font-weight:700; padding:3px 9px; border-radius:999px; }
.badge.replay { background:var(--mute); color:var(--text2); } .badge.live { background:var(--okBg); color:var(--ok); }
.modeline { color:var(--text2); font-size:13px; flex:1 1 280px; }
.shift { display:flex; flex-wrap:wrap; align-items:center; gap:8px 14px; margin-left:auto; }
.shift .stat { font-size:13px; color:var(--text2); } .shift .stat b { color:var(--text); font-variant-numeric:tabular-nums; }
.primary { background:var(--btn); color:var(--btnText); border:0; border-radius:8px; padding:9px 16px; font-weight:600; cursor:pointer; }
.secondary { background:var(--btn2); border:1px solid var(--border); border-radius:8px; padding:8px 14px; font-weight:600; cursor:pointer; }
.link { background:none; border:0; color:var(--text2); text-decoration:underline; cursor:pointer; padding:8px 4px; }
button:disabled { opacity:.55; cursor:default; }
.hint { font-size:12px; color:var(--label); }
main { max-width:1500px; margin:0 auto; padding:20px 24px; display:grid; grid-template-columns:minmax(0, 1.3fr) minmax(360px, 1fr); gap:20px; align-items:start; }
@media (max-width: 1080px) { main { grid-template-columns:minmax(0, 1fr); } }
@media (max-width: 640px) { main { padding:16px; } header { padding:12px 16px; } .shift { margin-left:0; } }
.panel { background:var(--card); border:1px solid var(--border); border-radius:12px; padding:16px; min-width:0; }
.left { display:flex; flex-direction:column; gap:20px; min-width:0; }
.head { display:flex; flex-wrap:wrap; justify-content:space-between; align-items:baseline; gap:4px 12px; margin-bottom:12px; }
h2 { margin:0; font-size:15px; } h3 { margin:0 0 6px; font-size:13px; color:var(--label); font-weight:600; text-transform:uppercase; letter-spacing:.04em; }
.legend { font-size:12px; color:var(--text2); } .legend b.p { color:#22c55e; } .legend b.v { color:#ff6a1f; }
.cams { display:grid; grid-template-columns:repeat(2, minmax(0, 1fr)); gap:12px; }
@media (max-width: 640px) { .cams { grid-template-columns:minmax(0, 1fr); } }
.cam { margin:0; border:1px solid var(--border); border-radius:10px; overflow:hidden; }
.cam figcaption { display:flex; justify-content:space-between; padding:7px 10px; font-size:13px; font-weight:600; }
.cam figcaption small { color:var(--label); font-weight:500; font-variant-numeric:tabular-nums; }
.screen { position:relative; aspect-ratio:16/9; background:#111214; }
.screen img { display:block; width:100%; height:100%; object-fit:cover; }
.words { position:absolute; inset:auto 0 0 0; padding:8px 10px; font-size:13px; color:#f2f2f2; background:linear-gradient(transparent, rgba(0,0,0,.75)); }
.words.full { inset:0; display:flex; align-items:center; justify-content:center; background:#111214; }
ol { list-style:none; margin:0; padding:0; display:flex; flex-direction:column; gap:6px; }
.row { width:100%; text-align:left; background:none; border:1px solid var(--border); border-radius:10px; padding:9px 12px; cursor:pointer; display:grid; grid-template-columns:1fr auto; gap:2px 10px; }
.row:hover { border-color:var(--label); } .row.on { background:var(--sel); border-color:var(--btn); }
.row b { font-size:13px; } .row span.sub { grid-column:1 / -1; color:var(--text2); font-size:13px; }
.tag { align-self:start; font-size:11px; font-weight:700; padding:3px 8px; border-radius:999px; white-space:nowrap; }
.t-waiting { background:var(--alertBg); color:var(--alert); } .t-signed { background:var(--okBg); color:var(--ok); }
.t-logged, .t-done, .t-closed { background:var(--warnBg); color:var(--warn); } .t-reviewing { background:var(--mute); color:var(--text2); }
.t-declined { background:var(--mute); color:var(--text2); } .t-failed { background:var(--alertBg); color:var(--alert); }
.empty { color:var(--text2); }
aside.panel { position:sticky; top:16px; max-height:calc(100vh - 32px); overflow:auto; display:flex; flex-direction:column; gap:18px; }
@media (max-width: 1080px) { aside.panel { position:static; max-height:none; } }
#d-body { display:flex; flex-direction:column; gap:18px; }
.d-head { display:flex; align-items:flex-start; gap:10px; } .d-head h2 { flex:1; font-size:16px; }
.frame { position:relative; border-radius:8px; overflow:hidden; background:#111214; aspect-ratio:16/9; }
.frame img { display:block; width:100%; height:100%; object-fit:contain; }
.box { position:absolute; border:2px solid; border-radius:3px; pointer-events:none; } .box.p { border-color:#22c55e; } .box.v { border-color:#ff6a1f; }
.strip { display:grid; grid-template-columns:repeat(auto-fill, minmax(64px, 1fr)); gap:6px; margin-top:8px; }
.strip button { padding:0; border:2px solid transparent; border-radius:6px; background:none; cursor:pointer; overflow:hidden; }
.strip button.on { border-color:var(--btn); } .strip img { display:block; width:100%; aspect-ratio:16/9; object-fit:cover; background:#111214; }
.strip span { display:block; font-size:11px; color:var(--label); padding:2px 0; }
.facts { display:grid; grid-template-columns:repeat(3, minmax(0, 1fr)); gap:8px; margin:12px 0 0; }
.facts div { background:var(--bg); border-radius:8px; padding:8px 10px; } .facts dt { font-size:11px; color:var(--label); } .facts dd { margin:0; font-weight:600; }
.review { margin-top:14px; } .review p { margin:4px 0; } .verdict { font-weight:700; }
.decide { border:1px solid var(--btn); border-radius:10px; padding:14px; background:var(--sel); }
.decide .lead { font-weight:700; font-size:15px; margin:2px 0 6px; } .decide p { margin:6px 0; }
.decide figure { margin:10px 0; } .decide figure img { width:100%; border-radius:8px; display:block; background:#111214; }
.decide figcaption { font-size:12px; color:var(--label); margin-top:4px; }
.actions { display:flex; flex-wrap:wrap; gap:8px; align-items:center; margin-top:10px; }
textarea { width:100%; border:1px solid var(--border); border-radius:8px; padding:8px 10px; background:var(--card); resize:vertical; }
.decline label { display:block; font-size:13px; color:var(--text2); margin:10px 0 4px; }
.answered { border-radius:10px; padding:10px 12px; font-weight:600; }
.answered.approved { background:var(--okBg); color:var(--ok); } .answered.declined, .answered.closed { background:var(--mute); color:var(--text2); }
.err { color:var(--alert); font-size:13px; margin:6px 0 0; }
.turn { display:flex; flex-direction:column; gap:6px; padding:8px 0; border-top:1px solid var(--border); } .turn:first-child { border-top:0; }
.msg { border-radius:10px; padding:8px 11px; max-width:92%; } .msg p { margin:2px 0 0; white-space:pre-wrap; }
.msg .who { font-size:11px; font-weight:700; color:var(--label); }
.msg.you { align-self:flex-end; background:var(--sel); } .msg.agent { align-self:flex-start; background:var(--bg); }
.muted { font-size:12px; color:var(--label); margin:0; }
#ask { display:flex; flex-direction:column; gap:8px; }
#ask .actions { margin:0; justify-content:space-between; flex-wrap:nowrap; } #ask .note { flex:1 1 auto; }
.note { font-size:13px; color:var(--text2); margin:0; } .note code { white-space:nowrap; font-size:12px; background:var(--mute); padding:1px 5px; border-radius:4px; }
.sr { position:absolute; width:1px; height:1px; overflow:hidden; clip:rect(0 0 0 0); }
footer { max-width:1500px; margin:0 auto; padding:4px 24px 24px; font-size:12px; color:var(--label); display:flex; flex-wrap:wrap; gap:6px 16px; justify-content:space-between; }
footer a { color:var(--text2); }
</style>
</head>
<body>
<header>
  <div class="brand"><i aria-hidden="true"></i>Warehouse Watch</div>
  <span class="badge replay" id="mode">Replay</span>
  <span class="modeline" id="modeline">A recorded shift played back: no key, no spend. Decisions work; new questions need a live run.</span>
  <div class="shift">
    <span class="stat"><b id="progress">0:00 of 5:00</b></span>
    <span class="stat" id="spend"></span>
    <button class="primary" id="start" type="button">Start the shift</button>
    <span class="hint" id="start-hint"></span>
  </div>
</header>
<main>
  <div class="left">
    <section class="panel" aria-labelledby="cams-h">
      <div class="head"><h2 id="cams-h">Cameras</h2><span class="legend">People found on this computer <b class="p">■</b> · vehicles Claude spotted <b class="v">■</b></span></div>
      <div class="cams" id="cams"></div>
    </section>
    <section class="panel" aria-labelledby="list-h">
      <div class="head"><h2 id="list-h">Close approaches</h2><span class="hint" id="counts"></span></div>
      <ol id="list"><li class="empty">No close approaches yet. The agent sleeps until a moving vehicle and a person come within 1.5 m.</li></ol>
    </section>
  </div>
  <aside class="panel" id="detail" aria-live="polite">
    <p class="empty" id="d-empty">Choose a close approach to see its frame, what the agent made of it, and anything waiting for your decision.</p>
    <div id="d-body" hidden>
      <div class="d-head"><h2 id="d-title"></h2><span class="tag" id="d-status"></span></div>
      <section id="d-found"></section>
      <section id="d-decision"></section>
      <section><h3>Conversation about this incident</h3><div id="d-convo"></div></section>
      <form id="ask">
        <label class="sr" for="ask-text">Ask about this incident</label>
        <textarea id="ask-text" rows="2"></textarea>
        <div class="actions"><p class="note" id="ask-note"></p><button class="primary" id="ask-send" type="submit">Send</button></div>
      </form>
    </div>
  </aside>
</main>
<footer><span id="source">Video and ground truth: NVIDIA PhysicalAI-SmartSpaces, CC BY 4.0 (synthetic; no real people).</span><a href="/ui/">Developer view</a></footer>
<script>
const $ = id => document.getElementById(id);
const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const mmss = ms => { const s = Math.floor(ms / 1000); return Math.floor(s / 60) + ':' + String(s % 60).padStart(2, '0'); };
const enc = encodeURIComponent;
let S = null, selected = null, shownList = '', keys = {}, card = null, declining = false, deciding = false, asking = false, askNote = '';
const shownCam = {};

async function getJson(url) {
  const r = await fetch(url, { cache: 'no-store' });
  if (!r.ok) throw new Error(url + ' answered ' + r.status);
  return r.json();
}
async function command(cmd) {
  const r = await fetch('/api/surface/commands', { method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ surface: 'web', origin: { kind: 'pointer' }, command: cmd }) });
  const body = await r.json().catch(() => ({}));
  return { status: r.status, ok: r.ok, body };
}
const problem = res => res.body.detail || res.body.title || ('the server answered ' + res.status);

function header(s) {
  const live = s.mode === 'live';
  $('mode').textContent = live ? 'Live' : 'Replay';
  $('mode').className = 'badge ' + s.mode;
  $('modeline').textContent = live ? 'Claude answers as you watch'
    : 'A recorded shift played back: no key, no spend. Decisions work; new questions need a live run.';
  $('spend').textContent = live ? `$${s.spendUsd.toFixed(2)} of $${s.capUsd.toFixed(2)}` : `$${s.spendUsd.toFixed(2)} when it was recorded`;
  $('progress').textContent = `${mmss(s.shift.atMs)} of ${mmss(s.shift.totalMs)}`;
  const b = $('start');
  b.disabled = s.shift.phase !== 'waiting';
  b.textContent = { waiting: 'Start the shift', running: 'Shift under way', finished: 'Shift finished' }[s.shift.phase];
  $('start-hint').textContent = s.shift.phase === 'finished' ? 'restart the app to run it again' : '';
  $('source').textContent = 'Video and ground truth: ' + s.source + ', NVIDIA, CC BY 4.0 (synthetic; no real people).';
}

function cameras(s) {
  const box = $('cams');
  if (!box.children.length) {
    box.innerHTML = s.cameras.map(c => `<figure class="cam"><figcaption>${esc(c.label)}<small id="t-${c.name}"></small></figcaption>
      <div class="screen"><img id="i-${c.name}" alt="${esc(c.label)}, the latest frame" hidden><div class="words full" id="w-${c.name}"></div></div></figure>`).join('');
    for (const c of s.cameras) {
      const img = $('i-' + c.name);
      img.addEventListener('load', () => { img.hidden = false; });
      img.addEventListener('error', () => { img.hidden = true; });
    }
  }
  for (const c of s.cameras) {
    const img = $('i-' + c.name), words = $('w-' + c.name);
    if (c.atMs != null && shownCam[c.name] !== c.atMs) {
      shownCam[c.name] = c.atMs;
      img.src = `/warehouse/camera/${c.name}.png?at=${c.atMs}`;
    } else if (c.atMs == null && c.poster && shownCam[c.name] === undefined) {
      shownCam[c.name] = 'poster';
      img.src = `/warehouse/camera/${c.name}.png?first`;
    }
    $('t-' + c.name).textContent = c.atMs != null ? mmss(c.atMs) : '';
    const text = s.shift.phase === 'waiting' ? 'Waiting for the shift to start'
      : (c.atMs == null ? (s.shift.phase === 'finished' ? 'No frames from this camera' : 'Starting the camera…') : '');
    words.textContent = text;
    words.hidden = !text;
    words.className = 'words' + (shownCam[c.name] === undefined ? ' full' : '');
  }
}

function list(s) {
  const parts = [`${s.raised} raised`, `${s.waiting} waiting for you`, `${s.logged} dismissed`];
  $('counts').textContent = s.incidents.length ? parts.join(' · ') : '';
  const key = JSON.stringify([s.incidents, selected]);
  if (key === shownList) return;
  shownList = key;
  if (!s.incidents.length) return;
  $('list').innerHTML = s.incidents.map(i => `<li><button type="button" class="row${i.id === selected ? ' on' : ''}" data-id="${esc(i.id)}">
    <b>${esc(i.id)} · ${esc(i.camera)} at ${esc(i.at)}</b><span class="tag t-${i.status}">${esc(i.statusText)}</span>
    <span class="sub">A person and a ${esc(i.vehicle)} ${i.gapM.toFixed(1)} m apart, the vehicle at ${i.speed.toFixed(1)} m/s</span></button></li>`).join('');
}
$('list').addEventListener('click', e => {
  const row = e.target.closest('button.row');
  if (row) select(row.dataset.id);
});

async function tick() {
  try { S = await getJson('/warehouse/state'); } catch (e) { return; }
  header(S); cameras(S); list(S);
}
$('start').addEventListener('click', async () => {
  $('start').disabled = true;
  await fetch('/warehouse/start', { method: 'POST' }).catch(() => {});
  tick();
});

function select(id) {
  if (selected !== id) { keys = {}; card = null; declining = false; askNote = ''; $('ask-text').value = ''; }
  selected = id;
  $('d-empty').hidden = true; $('d-body').hidden = false;
  if (S) list(S);
  if (window.matchMedia('(max-width: 1080px)').matches) $('detail').scrollIntoView({ behavior: 'smooth' });
  detail();
}

function set(id, key, html) {
  if (keys[id] === key) return false;
  keys[id] = key; $(id).innerHTML = html; return true;
}

async function detail() {
  const id = selected;
  if (!id) return;
  // An incident this server does not know, after a restart, closes the panel.
  if (S && !S.incidents.some(i => i.id === id)) {
    selected = null; $('d-body').hidden = true; $('d-empty').hidden = false; return;
  }
  let inc;
  try { inc = await getJson(`/warehouse/incidents/${enc(id)}`); } catch (e) { return; }
  let cards = [], entries = [];
  if (inc.turn) {
    const [d, t] = await Promise.all([
      getJson(`/api/surface/decisions?surface=web&session=${enc(id)}`).catch(() => ({ decisions: [] })),
      getJson(`/api/surface/sessions/${enc(id)}/thread?surface=web`).catch(() => ({ entries: [] })),
    ]);
    cards = d.decisions || []; entries = t.entries || [];
  }
  if (id !== selected) return;
  $('d-title').textContent = `${inc.id} · ${inc.camera} at ${inc.at}`;
  $('d-status').textContent = inc.statusText; $('d-status').className = 'tag t-' + inc.status;
  found(inc); decision(inc, cards); conversation(inc, entries); ask(inc, entries);
}

function found(inc) {
  const k = inc.keyIndex;
  const pct = (b, i) => [b[0] / inc.width * 100, b[1] / inc.height * 100, (b[2] - b[0]) / inc.width * 100, (b[3] - b[1]) / inc.height * 100][i].toFixed(2) + '%';
  const box = (b, cls) => `<div class="box ${cls}" style="left:${pct(b, 0)};top:${pct(b, 1)};width:${pct(b, 2)};height:${pct(b, 3)}"></div>`;
  const frames = inc.frames.length && k != null ? `<div class="frame" id="d-frame"><img id="d-img" src="/warehouse/incidents/${enc(inc.id)}/frames/${k}.png" alt="${esc(inc.camera)} at ${esc(inc.at)}, the closest approach">
      <div id="d-boxes">${box(inc.personBox, 'p')}${box(inc.vehicleBox, 'v')}</div></div>
    <div class="strip" id="d-strip">${inc.frames.map((f, i) => `<button type="button" data-k="${i}" class="${i === k ? 'on' : ''}" aria-label="Frame ${esc(f.at)}">
      <img src="/warehouse/incidents/${enc(inc.id)}/frames/${i}.png" alt="" loading="lazy"><span>${esc(f.at)}</span></button>`).join('')}</div>`
    : '<p class="empty">No frames were kept for this approach.</p>';
  let review;
  if (inc.review && inc.review.tool === 'raise_incident' && inc.card && inc.card.state === 'waiting') {
    review = '<p class="verdict">Raised as a near-miss</p><p>The agent put its report to you: see Your decision below.</p>';
  } else if (inc.review && inc.review.tool === 'raise_incident') {
    review = `<p class="verdict">Raised as a near-miss</p><p><b>${esc(inc.review.args.title)}</b></p><p>${esc(inc.review.args.detail)}</p>`;
  } else if (inc.review) {
    review = `<p class="verdict">Logged, not a near-miss</p><p>${esc(inc.review.args.line)}</p>`;
  } else if (inc.turn && inc.turn.state === 'failed') {
    review = `<p class="err">Claude could not review it: ${esc(inc.turn.error || 'the turn failed')}</p>`;
  } else if (inc.turn && inc.turn.state === 'completed') {
    review = '<p>The agent finished without raising or logging it; its reply is below.</p>';
  } else {
    review = '<p class="empty">The agent is reviewing the clip…</p>';
  }
  const facts = `<dl class="facts"><div><dt>Closest gap</dt><dd>${inc.gapM.toFixed(1)} m</dd></div>
    <div><dt>Vehicle</dt><dd>${esc(inc.vehicle)} at ${inc.speed.toFixed(1)} m/s</dd></div><div><dt>Where</dt><dd>${esc(inc.camera)} at ${esc(inc.at)}</dd></div></dl>`;
  if (set('d-found', JSON.stringify([inc.id, inc.frames.length]), `${frames}${facts}<div class="review" id="d-review"></div>`)) {
    const strip = $('d-strip');
    if (strip) strip.addEventListener('click', e => {
      const b = e.target.closest('button[data-k]');
      if (!b) return;
      for (const x of strip.children) x.classList.toggle('on', x === b);
      $('d-img').src = `/warehouse/incidents/${enc(inc.id)}/frames/${b.dataset.k}.png`;
      $('d-boxes').hidden = Number(b.dataset.k) !== k;
    });
  }
  set('d-review', review, `<h3>The agent's review</h3>${review}`);
}

function decision(inc, cards) {
  const waiting = cards.find(c => c.status === 'waiting' && c.call && c.call.tool === 'raise_incident');
  if (waiting) {
    card = waiting;
    const args = waiting.call.args || {};
    const img = waiting.media && waiting.media.images && waiting.media.images[0];
    const src = img ? (img.href || `/api/surface/decisions/${enc(waiting.id)}/evidence/0?surface=web`) : null;
    const html = `<div class="decide"><h3>Your decision</h3>
      <p class="lead">${esc(waiting.heading)}</p><p>${esc(args.detail)}</p>
      ${src ? `<figure><img src="${esc(src)}" alt="${esc(img.caption || 'The frame of the closest approach')}"><figcaption>${esc(img.caption || '')}</figcaption></figure>` : ''}
      <p class="hint">Approving signs off the near-miss: the agent files the incident report and a line in the shift log.</p>
      <div class="actions" id="dec-actions"><button class="primary" type="button" data-act="approve">Approve</button><button class="secondary" type="button" data-act="decline">Decline</button></div>
      <div class="decline" id="dec-why" hidden><label for="dec-reason">Why you decline it (optional; the agent reads it)</label><textarea id="dec-reason" rows="2"></textarea>
        <div class="actions"><button class="primary" type="button" data-act="confirm">Confirm decline</button><button class="link" type="button" data-act="cancel">Cancel</button></div></div>
      <p class="err" id="dec-err"></p></div>`;
    if (set('d-decision', waiting.id + ':' + waiting.version, html)) declining = false;
    $('dec-why').hidden = !declining;
    $('dec-actions').hidden = declining;
    for (const b of $('d-decision').querySelectorAll('button')) b.disabled = deciding;
    return;
  }
  card = null;
  const a = inc.card;
  if (a && a.state !== 'waiting') {
    const text = a.state === 'approved' ? `You approved at ${a.atText}: the near-miss is signed off.`
      : a.state === 'declined' ? `You declined at ${a.atText}${a.reason ? ': ' + a.reason : '.'}`
      : `Closed at ${a.atText} without a sign-off.`;
    set('d-decision', 'answered:' + text, `<h3>Your decision</h3><div class="answered ${a.state}">${esc(text)}</div>`);
  } else {
    set('d-decision', 'none', '');
  }
}
$('d-decision').addEventListener('click', async e => {
  const b = e.target.closest('button[data-act]');
  if (!b || !card) return;
  const act = b.dataset.act;
  if (act === 'decline' || act === 'cancel') { declining = act === 'decline'; $('dec-why').hidden = !declining; $('dec-actions').hidden = declining; if (declining) $('dec-reason').focus(); return; }
  const cmd = { type: 'decide', decisionId: card.id, version: card.version, expanded: true };
  if (act === 'approve') { cmd.answer = 'allowOnce'; cmd.evidence = ((card.media && card.media.images) || []).map(i => i.digest); }
  else { cmd.answer = 'deny'; const why = $('dec-reason').value.trim(); if (why) cmd.reason = why; }
  deciding = true;
  for (const x of $('d-decision').querySelectorAll('button')) x.disabled = true;
  const res = await command(cmd).catch(err => ({ ok: false, status: 0, body: { detail: String(err) } }));
  deciding = false;
  if (res.status === 409 || (res.ok && (res.body.type === 'changed' || res.body.type === 'heldAgain'))) {
    keys['d-decision'] = null; await detail();
    if ($('dec-err')) $('dec-err').textContent = 'The report changed while you read it. Read it again, then decide.';
    return;
  }
  if (!res.ok) {
    $('dec-err').textContent = 'Your decision was not recorded: ' + problem(res);
    for (const x of $('d-decision').querySelectorAll('button')) x.disabled = false;
    return;
  }
  declining = false;
  detail(); tick();
});

function turns(inc, entries) {
  const out = []; let t = null;
  for (const e of entries) for (const it of e.items) {
    if (it.kind === 'message' && it.from === 'you') {
      t = { start: it.text, first: e.at, last: e.at, steps: 0, reply: null, waited: 0, held: null }; out.push(t); continue;
    }
    if (!t) { t = { start: null, first: e.at, last: e.at, steps: 0, reply: null, waited: 0, held: null }; out.push(t); }
    t.last = e.at;
    // The time a held call waited for the supervisor is not time the agent worked.
    if (t.held && it.kind !== 'decision') { t.waited += Date.parse(e.at) - Date.parse(t.held); t.held = null; }
    if (it.kind === 'decision' && !t.held) t.held = e.at;
    if (it.kind === 'message') t.reply = it.text;
    else if (it.kind !== 'suggestions') { if (it.kind === 'activity') t.steps++; t.reply = null; }
  }
  return out;
}

function conversation(inc, entries) {
  const all = turns(inc, entries);
  const state = inc.turn ? inc.turn.state : null;
  const html = all.map((t, n) => {
    const lastTurn = n === all.length - 1;
    const who = t.start == null ? '' : t.start === inc.task
      ? `<p class="muted">Started by the floor watch at ${esc(inc.at)}: a person and a ${esc(inc.vehicle)} ${inc.gapM.toFixed(1)} m apart.</p>`
      : `<div class="msg you"><span class="who">You</span><p>${esc(t.start)}</p></div>`;
    const secs = Math.max(1, Math.round((Date.parse(t.last) - Date.parse(t.first) - t.waited) / 1000));
    const open = lastTurn && !t.reply && (state === 'running' || state === 'queued');
    const steps = t.steps ? `<p class="muted">${open ? 'Working' : `Worked for ${secs} s`} · ${t.steps} ${t.steps === 1 ? 'step' : 'steps'}</p>` : '';
    const reply = t.reply ? `<div class="msg agent"><span class="who">Warehouse Watch</span><p>${esc(t.reply)}</p></div>` : '';
    const waits = lastTurn && !t.reply && inc.card && inc.card.state === 'waiting' ? '<p class="muted">Waiting for your decision above.</p>' : '';
    const failed = lastTurn && state === 'failed' ? `<p class="err">Claude could not answer: ${esc(inc.turn.error || 'the turn failed')}</p>` : '';
    return `<div class="turn">${who}${steps}${reply}${waits}${failed}</div>`;
  }).join('') || '<p class="empty">Nothing yet.</p>';
  set('d-convo', html, html);
}

function ask(inc, entries) {
  const live = S && S.mode === 'live';
  const box = $('ask-text'), send = $('ask-send'), note = $('ask-note');
  box.placeholder = `Ask about ${inc.id}…`;
  const state = inc.turn ? inc.turn.state : null;
  const all = turns(inc, entries), last = all[all.length - 1];
  const running = (state === 'running' || state === 'queued') && !(last && last.reply);
  const person = last && last.start != null && last.start !== inc.task;
  if (!live) {
    box.disabled = true; send.disabled = true;
    note.innerHTML = 'Questions need a live run: set <code>ANTHROPIC_API_KEY</code> and start with <code>serve --live</code>.';
    return;
  }
  box.disabled = false; send.disabled = asking;
  if (askNote) note.textContent = askNote;
  else if (running || asking) note.textContent = person || asking ? 'Warehouse Watch is answering…' : 'Warehouse Watch is reviewing the clip…';
  else note.textContent = '';
}
$('ask').addEventListener('submit', async e => {
  e.preventDefault();
  const text = $('ask-text').value.trim();
  if (!text || !selected || !S || S.mode !== 'live') return;
  asking = true; askNote = '';
  $('ask-send').disabled = true; $('ask-note').textContent = 'Warehouse Watch is answering…';
  const res = await command({ type: 'send', sessionId: selected, text }).catch(err => ({ ok: false, status: 0, body: { detail: String(err) } }));
  asking = false;
  if (!res.ok) askNote = 'Your question was not sent: ' + problem(res);
  else { $('ask-text').value = ''; if (res.body.type === 'queued') askNote = res.body.text || 'Sent after this card is answered'; }
  detail();
});
$('ask-text').addEventListener('keydown', e => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) $('ask').requestSubmit(); });

tick(); setInterval(tick, 700);
setInterval(detail, 1500);
</script>
</body>
</html>
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_cameras_as_the_floor_does() {
        assert_eq!(label("Camera_0001"), "Camera 1");
        assert_eq!(label("Camera_0015"), "Camera 15");
    }

    #[test]
    fn an_incident_reads_as_the_agent_left_it() {
        assert_eq!(
            status(Some("shift_log"), None, None),
            ("logged", "Logged, not a near-miss")
        );
        assert_eq!(
            status(Some("raise_incident"), None, None),
            ("reviewing", "Reviewing")
        );
        assert_eq!(status(None, None, None), ("reviewing", "Reviewing"));
    }
}
