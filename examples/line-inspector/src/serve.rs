//! The line page while the line runs at one tray a second, once the engineer starts it: the
//! latest tray, the shift's counts, the reject rate and the reject log, and for a chosen part
//! what the line and the agent found, the engineer's decision, the part's conversation and a
//! box to ask about it. The web home at `/ui/` stays as the developer view.

use std::collections::BTreeMap;
use std::future::IntoFuture;
use std::sync::Arc;
use std::time::Duration;

use adk_artifact::ArtifactService;
use adk_core::{
    ApprovalAction, ApprovalRequest, ApprovalState, InboxQuery, RequestContext, RunRecord,
    RunState, SingleAgentLoader,
};
use adk_kernel::{ProcessFilter, ProcessKind};
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

use crate::agent::{APP, USER};
use crate::line::TrayFeed;
use crate::tape::Mode;
use crate::{App, Common};

struct Engineer;

#[async_trait]
impl RequestContextExtractor for Engineer {
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

struct PartCards;

#[async_trait]
impl CardHints for PartCards {
    async fn hint(&self, request: &adk_core::ApprovalRequest) -> Option<CardHint> {
        (request.tool_name == "ask_engineer").then(|| {
            CardHint::new().with_heading(
                request.args_redacted["title"]
                    .as_str()
                    .unwrap_or("A part to decide")
                    .to_string(),
            )
        })
    }
}

/// Where the shift is: the line waits for **Start the line**, runs, and ends.
#[derive(Clone, Debug, PartialEq)]
enum Phase {
    Waiting,
    Running,
    Finished,
    Stopped(String),
}

/// What the page's routes share.
struct Site {
    app: Arc<App>,
    phase: watch::Sender<Phase>,
    daily_usd: f64,
    models: Value,
}

pub async fn serve(common: Common, port: u16) -> anyhow::Result<()> {
    let app = Arc::new(App::build(&common).await?);
    app.tape.explain_misses();
    let desk = Arc::clone(&app.desk);
    desk.kernel.start().await?;
    app.dispatch().await;
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
        .agent_name(APP, "Line Inspector")
        .card_hints(Arc::new(PartCards))
        .build();
    let config = ServerConfig::new(
        Arc::new(SingleAgentLoader::new(desk.agent.clone())),
        desk.sessions.clone(),
    )
    .with_request_context(Arc::new(Engineer));
    let site = Arc::new(Site {
        app: Arc::clone(&app),
        phase: watch::Sender::new(Phase::Waiting),
        daily_usd: common.daily_usd,
        models: json!({
            "agent": common.agent_model,
            "reasoner": common.reasoner_model,
            "second": common.second_model,
        }),
    });
    let page = axum::Router::new()
        .route("/line", get(|| async { Html(PAGE) }))
        .route("/line/state", get(state))
        .route("/line/start", post(start))
        .route("/line/tray/{id}", get(tray_image))
        .route("/line/part/{id}", get(part))
        .route("/line/part/{id}/crop.png", get(part_crop))
        .with_state(Arc::clone(&site));
    let router = ServerBuilder::new(config)
        .add_api_routes(adk_surface::web::router(engine))
        .add_root_routes(adk_surface::web::home().merge(page))
        .build();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("Line Inspector on http://127.0.0.1:{port}/line: press Start the line there");
    println!("the developer view on http://127.0.0.1:{port}/ui/");
    let stopping = Arc::new(tokio::sync::Notify::new());
    let server = tokio::spawn(
        axum::serve(listener, router)
            .with_graceful_shutdown({
                let stopping = Arc::clone(&stopping);
                async move { stopping.notified().await }
            })
            .into_future(),
    );
    let running = {
        let site = Arc::clone(&site);
        tokio::spawn(async move {
            let mut phase = site.phase.subscribe();
            if phase.wait_for(|p| *p == Phase::Running).await.is_err() {
                return;
            }
            let app = &site.app;
            match crate::line::run(
                TrayFeed::new(app.trays.clone(), true),
                Arc::clone(&app.inspector),
                Arc::clone(&app.line),
            )
            .await
            {
                Ok(_) => {
                    println!(
                        "[line] the shift's trays are through; the page stays open until Ctrl-C"
                    );
                    site.phase.send_replace(Phase::Finished);
                }
                Err(err) => {
                    eprintln!("[line] stopped: {err}");
                    site.phase.send_replace(Phase::Stopped(err.to_string()));
                }
            }
        })
    };
    tokio::signal::ctrl_c().await?;
    running.abort();
    stopping.notify_one();
    let _ = tokio::time::timeout(Duration::from_secs(3), server).await;
    desk.kernel.stop().await?;
    app.tape.save()?;
    Ok(())
}

/// **Start the line**: the feed starts once, whoever presses it first.
async fn start(State(site): State<Arc<Site>>) -> impl IntoResponse {
    let started = site.phase.send_if_modified(|phase| {
        let waiting = *phase == Phase::Waiting;
        if waiting {
            *phase = Phase::Running;
        }
        waiting
    });
    axum::Json(json!({ "started": started }))
}

/// The latest request put to the engineer in each part's session.
async fn decisions(app: &App) -> BTreeMap<String, ApprovalRequest> {
    let query = InboxQuery::default()
        .with_app_name(APP)
        .with_user_id(USER)
        .with_states([
            ApprovalState::Pending,
            ApprovalState::Approved,
            ApprovalState::Rejected,
            ApprovalState::Responded,
            ApprovalState::Expired,
            ApprovalState::Cancelled,
        ])
        .with_limit(500);
    let mut latest = BTreeMap::new();
    if let Ok(page) = app.desk.approvals.inbox(query).await {
        for request in page.items {
            latest.insert(request.scope.session_id.clone(), request);
        }
    }
    latest
}

/// The latest run in each part's session, with whether a person's message started it.
async fn turns(app: &App) -> BTreeMap<String, (RunRecord, bool)> {
    let mut latest: BTreeMap<String, (RunRecord, bool)> = BTreeMap::new();
    let Ok(processes) = app
        .desk
        .kernel
        .processes()
        .ps(ProcessFilter::default())
        .await
    else {
        return latest;
    };
    for process in processes {
        let Some(session) = process.run.session_id.clone() else {
            continue;
        };
        if latest
            .get(&session)
            .is_none_or(|(run, _)| run.created_at <= process.run.created_at)
        {
            let asked = matches!(process.meta.kind, ProcessKind::Interactive);
            latest.insert(session, (process.run, asked));
        }
    }
    latest
}

/// A part's call on the page: the line's or the agent's, or the engineer's once they answer
/// a part put to them. Declining the suggestion overrules it.
fn final_call(verdict: String, decision: Option<&ApprovalRequest>) -> String {
    let Some(decision) = decision.filter(|_| verdict == "engineer") else {
        return verdict;
    };
    let suggested = decision.args_redacted["suggestion"]
        .as_str()
        .unwrap_or("reject");
    let overruled = if suggested == "pass" {
        "reject"
    } else {
        "pass"
    };
    match decision.state {
        ApprovalState::Approved => suggested.to_string(),
        ApprovalState::Rejected => overruled.to_string(),
        ApprovalState::Pending
        | ApprovalState::Responded
        | ApprovalState::Expired
        | ApprovalState::Cancelled => verdict,
    }
}

/// Where a part stands, in the reject log's words: a short tag, its tone and a sentence.
fn standing(
    part: &crate::line::Part,
    agent: Option<&Value>,
    decision: Option<&ApprovalRequest>,
    turn: Option<&(RunRecord, bool)>,
) -> (&'static str, &'static str, String) {
    if part.stage == "reject" {
        return ("Rejected", "reject", "Rejected on this computer".into());
    }
    if let Some(decision) = decision {
        let suggested = decision.args_redacted["suggestion"]
            .as_str()
            .unwrap_or("reject")
            .to_string();
        return match decision.state {
            ApprovalState::Pending => (
                "Waiting for you",
                "waiting",
                format!("The models disagree; Line Inspector suggests {suggested}"),
            ),
            ApprovalState::Approved => (
                "You approved",
                "decided",
                format!("You approved Line Inspector's {suggested}"),
            ),
            ApprovalState::Rejected | ApprovalState::Responded => (
                "You declined",
                "decided",
                format!(
                    "You declined Line Inspector's {suggested}: the part {}",
                    if suggested == "pass" {
                        "is rejected"
                    } else {
                        "passes"
                    }
                ),
            ),
            ApprovalState::Expired | ApprovalState::Cancelled => (
                "Not decided",
                "muted",
                "The decision ended without an answer".into(),
            ),
        };
    }
    if let Some(agent) = agent {
        return if agent["verdict"] == "reject" {
            ("Rejected", "reject", "Rejected on a second look".into())
        } else {
            ("Passed", "pass", "Passed on a second look".into())
        };
    }
    match turn.map(|(run, _)| run.state) {
        Some(RunState::Failed | RunState::Cancelled) => (
            "Look failed",
            "reject",
            "The second look stopped before a verdict".into(),
        ),
        Some(RunState::Completed) => (
            "No verdict",
            "muted",
            "The second look ended without a verdict".into(),
        ),
        _ => (
            "Looking",
            "muted",
            "Line Inspector is taking a second look".into(),
        ),
    }
}

async fn state(State(site): State<Arc<Site>>) -> impl IntoResponse {
    let app = &site.app;
    let seen = app
        .book
        .trays_seen
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let parts = app.book.all();
    let decided = decisions(app).await;
    let turns = turns(app).await;
    let agent = app
        .desk
        .verdicts
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let (mut rejects, mut looks) = (0, 0);
    let mut blocks: Vec<(usize, usize)> = Vec::new();
    let mut log = Vec::new();
    let mut waiting = Vec::new();
    // The latest tray's calls: its image is drawn again when a candle is decided.
    let mut latest_key = seen.last().cloned().unwrap_or_default();
    for part in &parts {
        let v = final_call(crate::verdict(app, part).await, decided.get(&part.id));
        let tray_no: usize = part.tray[1..].parse().unwrap_or(1);
        let block = (tray_no - 1) / 10;
        if blocks.len() <= block {
            blocks.resize(block + 1, (0, 0));
        }
        blocks[block].1 += 1;
        if v == "reject" {
            rejects += 1;
            blocks[block].0 += 1;
        }
        if part.stage == "borderline" {
            looks += 1;
        }
        if seen.last() == Some(&part.tray) {
            latest_key.push_str(&format!(":{v}"));
        }
        if part.stage == "pass" {
            continue;
        }
        let decision = decided.get(&part.id);
        if decision.is_some_and(|d| d.state == ApprovalState::Pending) {
            waiting.push(part.id.clone());
        }
        let (tag, tone, said) = standing(part, agent.get(&part.id), decision, turns.get(&part.id));
        log.push(json!({
            "id": part.id,
            "score": part.score,
            "stage": part.stage,
            "mm": part.longest_mm,
            "tag": tag,
            "tone": tone,
            "said": said,
        }));
    }
    log.reverse();
    let phase = site.phase.borrow().clone();
    // An empty sum is -0.0; adding 0.0 makes it 0.0.
    let spend: f64 = app.tape.spend().values().map(|s| s.usd).sum::<f64>() + 0.0;
    axum::Json(json!({
        "phase": match &phase {
            Phase::Waiting => "waiting",
            Phase::Running => "running",
            Phase::Finished => "finished",
            Phase::Stopped(_) => "stopped",
        },
        "stopped": match &phase {
            Phase::Stopped(reason) => Some(reason.clone()),
            Phase::Waiting | Phase::Running | Phase::Finished => None,
        },
        "mode": if app.tape.mode == Mode::Live { "live" } else { "replay" },
        "latest": seen.last(),
        "latestKey": latest_key,
        "trays": seen.len(),
        "totalTrays": app.trays.len(),
        "rejects": rejects,
        "looks": looks,
        "waiting": waiting,
        "blocks": blocks.iter().map(|(r, n)| json!({ "rejected": r, "candles": n })).collect::<Vec<_>>(),
        "log": log,
        "spendUsd": (spend * 10_000.0).round() / 10_000.0,
        "dailyUsd": site.daily_usd,
        "models": site.models,
    }))
}

/// One part for the detail panel: what the line found, the agent's verdict, where the
/// engineer's decision stands and the latest turn in its session.
async fn part(State(site): State<Arc<Site>>, Path(id): Path<String>) -> Response {
    let app = &site.app;
    let Some(part) = app.book.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let agent = app
        .desk
        .verdicts
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
        .cloned();
    let decision = decisions(app).await.remove(&id);
    let turn = turns(app).await.remove(&id);
    let (tag, tone, said) = standing(&part, agent.as_ref(), decision.as_ref(), turn.as_ref());
    let decided = decision
        .as_ref()
        .and_then(|request| request.decisions.last());
    let candle: usize = part
        .id
        .rsplit('p')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(1);
    axum::Json(json!({
        "id": part.id,
        "tray": part.tray,
        "candle": candle,
        "score": part.score,
        "stage": part.stage,
        "longestMm": part.longest_mm,
        "areaMm2": part.area_mm2,
        "defect": part.defect,
        "defective": part.defective,
        "tag": tag,
        "tone": tone,
        "said": said,
        "hasSession": app.desk.started().contains(&part.id),
        "agent": agent,
        "decision": decision.as_ref().map(|request| json!({
            "args": request.args_redacted,
            "answer": decided.map(|d| match d.kind {
                ApprovalAction::Approve | ApprovalAction::Edit => "approved",
                ApprovalAction::Reject | ApprovalAction::Respond => "declined",
            }),
            "reason": decided.and_then(|d| d.reason.clone()),
        })),
        "turn": turn.map(|(run, asked)| json!({
            "state": match run.state {
                RunState::Waiting => "waiting",
                RunState::Completed => "done",
                RunState::Failed | RunState::Cancelled => "failed",
                // Queued, Running, and any state `RunState` adds later.
                _ => "running",
            },
            "asked": asked,
            "error": run.error,
            "at": run.updated_at,
        })),
    }))
    .into_response()
}

/// The part cut from its tray: its least familiar area boxed in amber and, for a reject, the
/// segmented defect tinted red.
async fn part_crop(State(site): State<Arc<Site>>, Path(id): Path<String>) -> Response {
    let app = &site.app;
    let Some(part) = app.book.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(tray) = app.trays.iter().find(|t| t.id == part.tray).cloned() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let region = app
        .book
        .regions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
        .cloned();
    let png = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
        let image = image::open(&tray.image)?.to_rgb8();
        let [x0, y0, x1, y1] = part.candle;
        let pad = (x1 - x0).max(y1 - y0) * 0.12;
        let left = (x0 - pad).max(0.0) as u32;
        let top = (y0 - pad).max(0.0) as u32;
        let right = ((x1 + pad) as u32).min(image.width());
        let bottom = ((y1 + pad) as u32).min(image.height());
        let mut crop = image::imageops::crop_imm(
            &image,
            left,
            top,
            right.saturating_sub(left).max(1),
            bottom.saturating_sub(top).max(1),
        )
        .to_image();
        if let Some(mask) = region {
            let bits = mask.decode();
            for (x, y, pixel) in crop.enumerate_pixels_mut() {
                let (gx, gy) = (x + left, y + top);
                if gx < mask.width && gy < mask.height && bits[(gy * mask.width + gx) as usize] {
                    let [r, g, b] = pixel.0;
                    *pixel = image::Rgb([
                        ((u16::from(r) + 239 * 2) / 3) as u8,
                        ((u16::from(g) + 68 * 2) / 3) as u8,
                        ((u16::from(b) + 68 * 2) / 3) as u8,
                    ]);
                }
            }
        }
        let [sx0, sy0, sx1, sy1] = part.suspect;
        let (sx0, sy0) = (
            (sx0 - left as f32).max(0.0) as u32,
            (sy0 - top as f32).max(0.0) as u32,
        );
        let (sx1, sy1) = (
            ((sx1 - left as f32).max(0.0) as u32).min(crop.width().saturating_sub(1)),
            ((sy1 - top as f32).max(0.0) as u32).min(crop.height().saturating_sub(1)),
        );
        let amber = image::Rgb([245, 194, 107]);
        for t in 0..4u32 {
            for x in sx0..=sx1 {
                for y in [sy0 + t, sy1.saturating_sub(t)] {
                    if y < crop.height() {
                        crop.put_pixel(x, y, amber);
                    }
                }
            }
            for y in sy0..=sy1 {
                for x in [sx0 + t, sx1.saturating_sub(t)] {
                    if x < crop.width() {
                        crop.put_pixel(x, y, amber);
                    }
                }
            }
        }
        let width = 360u32;
        let height = (crop.height() * width / crop.width().max(1)).max(1);
        let crop =
            image::imageops::resize(&crop, width, height, image::imageops::FilterType::Triangle);
        let mut out = Vec::new();
        crop.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
        Ok(out)
    })
    .await;
    match png {
        Ok(Ok(bytes)) => ([(header::CONTENT_TYPE, "image/png")], bytes).into_response(),
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// A tray with each candle ringed: green passed, red rejected, amber on a second look.
async fn tray_image(State(site): State<Arc<Site>>, Path(id): Path<String>) -> Response {
    let app = &site.app;
    let id = id.trim_end_matches(".png").to_string();
    let Some(tray) = app.trays.iter().find(|t| t.id == id).cloned() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let parts: Vec<_> = app
        .book
        .all()
        .into_iter()
        .filter(|p| p.tray == id)
        .collect();
    let decided = decisions(app).await;
    let mut verdicts = Vec::new();
    for part in &parts {
        let v = final_call(crate::verdict(app, part).await, decided.get(&part.id));
        verdicts.push((part.candle, v));
    }
    let png = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
        let image = image::open(&tray.image)?
            .resize(642, 584, image::imageops::FilterType::Triangle)
            .to_rgb8();
        let mut image = image;
        let scale = 642.0 / 1284.0;
        for (b, v) in verdicts {
            let rgb = match v.as_str() {
                "pass" => [34, 197, 94],
                "reject" => [239, 68, 68],
                _ => [245, 194, 107],
            };
            let (cx, cy) = ((b[0] + b[2]) / 2.0 * scale, (b[1] + b[3]) / 2.0 * scale);
            let r = (b[2] - b[0]) / 2.0 * scale - 4.0;
            for step in 0..720 {
                let a = step as f32 / 720.0 * std::f32::consts::TAU;
                for t in 0..3 {
                    let (x, y) = (
                        (cx + (r + t as f32) * a.cos()) as u32,
                        (cy + (r + t as f32) * a.sin()) as u32,
                    );
                    if x < image.width() && y < image.height() {
                        image.put_pixel(x, y, image::Rgb(rgb));
                    }
                }
            }
        }
        let mut out = Vec::new();
        image.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
        Ok(out)
    })
    .await;
    match png {
        Ok(Ok(bytes)) => (
            [
                (header::CONTENT_TYPE, "image/png"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response(),
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

const PAGE: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="icon" href="data:,">
<title>Line Inspector</title>
<style>
:root { --bg:#fafaf9; --card:#fff; --sunk:#f4f3f0; --border:#e7e6e2; --text:#18181b; --text2:#45454d; --label:#6b6b74;
  --btn:#c2410c; --btnText:#fff; --btn2:#f4f3f0; --alert:#b42318; --alertBg:#fdecea; --ok:#1e6b45; --okBg:#e7f4ec;
  --warn:#8a4b00; --warnBg:#fdf3e1; --info:#1f4f8a; --infoBg:#e8f0fb; --bar:#c2410c; --grid:#ecebe7; --axis:#8e8e97; --focus:#2563eb; }
@media (prefers-color-scheme: dark) { :root { --bg:#0e0f11; --card:#16171a; --sunk:#1d1e22; --border:#2a2b31; --text:#ededef; --text2:#c2c2ca; --label:#8e8e97;
  --btn:#ff6a1f; --btnText:#0e0f11; --btn2:#24252a; --alert:#ff8a80; --alertBg:#3a1d1b; --ok:#7ad3a0; --okBg:#17301f;
  --warn:#f5c26b; --warnBg:#3a2c12; --info:#9cc2f5; --infoBg:#172a44; --bar:#ff6a1f; --grid:#24252a; --axis:#6e6e77; --focus:#7aa7ff; } }
* { box-sizing: border-box; }
[hidden] { display: none !important; }
body { margin:0; font: 14px/1.45 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif; background:var(--bg); color:var(--text); }
button, textarea { font: inherit; color: inherit; }
:focus-visible { outline: 2px solid var(--focus); outline-offset: 2px; }
header { display:flex; flex-wrap:wrap; align-items:center; gap:10px 24px; padding:12px 24px; border-bottom:1px solid var(--border); background:var(--card); }
.brand { display:flex; align-items:center; gap:10px; }
.brand .mark { width:30px; height:30px; border-radius:8px; background:#f74c00; flex:none; }
.brand b { display:block; font-size:16px; } .brand small { color:var(--label); font-size:12px; }
.mode { display:flex; flex-wrap:wrap; align-items:center; gap:4px 10px; flex:1 1 420px; min-width:0; font-size:13px; color:var(--text2); }
.badge { font-size:11px; font-weight:700; letter-spacing:.04em; text-transform:uppercase; padding:3px 9px; border-radius:999px; }
.badge.replay { background:var(--infoBg); color:var(--info); } .badge.live { background:var(--okBg); color:var(--ok); }
.spend { color:var(--label); }
.run { display:flex; flex-wrap:wrap; align-items:center; gap:8px 12px; margin-left:auto; }
#progress { font-weight:600; font-variant-numeric: tabular-nums; }
.run small { flex-basis:100%; text-align:right; color:var(--label); font-size:12px; }
.run small:empty { display:none; }
button.primary { background:var(--btn); color:var(--btnText); border:0; border-radius:8px; padding:9px 16px; font-weight:600; cursor:pointer; }
button.secondary { background:var(--btn2); border:1px solid var(--border); border-radius:8px; padding:8px 16px; font-weight:600; cursor:pointer; }
button.danger { background:var(--alert); color:var(--card); border:0; border-radius:8px; padding:8px 14px; font-weight:600; cursor:pointer; }
button.link { background:none; border:0; padding:8px 6px; color:var(--text2); text-decoration:underline; cursor:pointer; }
button:disabled { opacity:.55; cursor:default; }
button.pill { background:var(--warnBg); color:var(--warn); border:0; border-radius:999px; padding:6px 12px; font-weight:600; font-size:13px; cursor:pointer; }
main { max-width:1480px; margin:0 auto; padding:20px 24px; display:grid; gap:20px; align-items:start;
  grid-template-columns: minmax(0,1.05fr) minmax(0,.8fr) minmax(0,1.15fr); grid-template-areas: "a log detail"; }
.col-a { grid-area:a; display:flex; flex-direction:column; gap:20px; min-width:0; }
.panel { background:var(--card); border:1px solid var(--border); border-radius:12px; padding:16px; min-width:0; }
.panel h2 { margin:0 0 10px; font-size:15px; }
h3 { margin:18px 0 8px; font-size:13px; font-weight:700; color:var(--text2); }
.tray img { width:100%; border-radius:8px; display:block; background:#000; aspect-ratio: 642 / 584; }
.empty { border:1px dashed var(--border); border-radius:10px; background:var(--sunk); color:var(--text2); padding:28px 20px; text-align:center; }
.tray .empty { aspect-ratio: 642 / 584; display:flex; align-items:center; justify-content:center; }
.legend { margin:8px 0 0; font-size:12px; color:var(--label); display:flex; gap:14px; flex-wrap:wrap; }
.dot { display:inline-block; width:10px; height:10px; border-radius:50%; margin-right:5px; vertical-align:-1px; border:2px solid; }
.dot.pass { border-color:#22c55e; } .dot.reject { border-color:#ef4444; } .dot.look { border-color:#f5c26b; }
.kpis { display:grid; grid-template-columns: repeat(4, minmax(0,1fr)); gap:10px; }
.kpis b { display:block; font-size:22px; font-variant-numeric: tabular-nums; } .kpis span { font-size:12px; color:var(--label); }
#chart { width:100%; height:auto; display:block; }
#chart .grid { stroke:var(--grid); stroke-width:1; } #chart .base { stroke:var(--axis); stroke-width:1; }
#chart .axis { fill:var(--label); font-size:10px; } #chart .val { fill:var(--text2); font-size:10.5px; font-weight:600; }
#chart .bar { fill:var(--bar); } #chart .hit { fill:transparent; } #chart g:hover .bar { opacity:.8; }
.log { grid-area:log; }
.log p.sub { margin:-4px 0 10px; font-size:12px; color:var(--label); }
ol#log { list-style:none; margin:0; padding:0; max-height: calc(100vh - 190px); overflow:auto; }
.row { width:100%; display:flex; flex-wrap:wrap; gap:4px 8px; align-items:center; text-align:left; background:none; border:0; border-top:1px solid var(--border); padding:9px 6px; cursor:pointer; border-radius:0; }
.row:hover { background:var(--sunk); } .row.on { background:var(--sunk); box-shadow: inset 3px 0 0 var(--btn); }
.row .meta { color:var(--label); font-size:12px; flex-basis:100%; }
.tag { font-size:11px; font-weight:700; padding:3px 8px; border-radius:999px; white-space:nowrap; }
.tag.reject { background:var(--alertBg); color:var(--alert); } .tag.pass { background:var(--okBg); color:var(--ok); }
.tag.waiting { background:var(--warnBg); color:var(--warn); } .tag.decided { background:var(--infoBg); color:var(--info); }
.tag.muted { background:var(--sunk); color:var(--text2); }
.detail { grid-area:detail; position:sticky; top:12px; max-height: calc(100vh - 24px); overflow:auto; }
.dhead { display:flex; align-items:flex-start; gap:10px; }
.dhead > div { flex:1; min-width:0; } .dhead h2 { margin:0; font-size:18px; } .dhead p { margin:2px 0 0; color:var(--text2); font-size:13px; }
button.icon { background:none; border:1px solid var(--border); border-radius:8px; width:30px; height:30px; cursor:pointer; color:var(--label); font-size:16px; line-height:1; }
figure { margin:0 0 10px; } figure img { display:block; width:100%; max-width:360px; height:auto; border-radius:8px; background:#000; }
figcaption { font-size:12px; color:var(--label); margin-top:4px; }
dl { margin:0; display:grid; grid-template-columns: minmax(110px, 34%) 1fr; gap:6px 12px; font-size:13px; }
dt { color:var(--label); } dd { margin:0; }
.card-d { border:1px solid var(--warn); background:var(--warnBg); border-radius:10px; padding:14px; }
.card-d.done { border-color:var(--border); background:var(--sunk); }
.card-d .q { margin:0 0 6px; font-weight:600; font-size:15px; } .card-d .proposal { margin:0 0 12px; }
.card-d dl { margin-bottom:12px; } .card-d figure img { max-width:240px; }
.card-d .fine, .fine { margin:8px 0 0; font-size:12px; color:var(--label); }
.actions { display:flex; gap:8px; flex-wrap:wrap; align-items:center; }
.declining label { display:block; font-size:13px; margin-bottom:4px; }
textarea { width:100%; border:1px solid var(--border); border-radius:8px; padding:8px 10px; background:var(--card); resize:vertical; }
.err { color:var(--alert); font-size:13px; margin:8px 0 0; }
.sys { color:var(--label); font-size:12px; margin:6px 0; }
.msg { margin:8px 0; padding:10px 12px; border-radius:10px; }
.msg p { margin:2px 0 0; white-space:pre-wrap; overflow-wrap:anywhere; }
.msg.you { background:var(--sunk); margin-left:15%; } .msg.agent { border:1px solid var(--border); margin-right:10%; }
.who { font-size:12px; font-weight:600; color:var(--text2); }
details.steps { color:var(--label); font-size:12px; margin:4px 0; } details.steps summary { cursor:pointer; }
details.steps ol { margin:4px 0 0; padding-left:18px; }
.status { margin:10px 0 0; font-size:13px; color:var(--text2); } .status.fail { color:var(--alert); }
form#ask { margin-top:14px; border-top:1px solid var(--border); padding-top:12px; }
form#ask label { display:block; font-weight:600; font-size:13px; margin-bottom:6px; }
.askrow { display:flex; gap:8px; align-items:flex-end; } .askrow textarea { flex:1; }
#ask-note { margin:8px 0 0; font-size:12px; color:var(--label); }
code { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size:12px; background:var(--sunk); padding:1px 4px; border-radius:4px; white-space:nowrap; }
footer { display:flex; flex-wrap:wrap; gap:6px 18px; justify-content:space-between; padding:12px 24px; font-size:12px; color:var(--label); border-top:1px solid var(--border); }
footer a { color:var(--text2); }
@media (max-width: 1239px) { main { grid-template-columns: minmax(0,1fr) minmax(0,1fr); grid-template-areas: "a detail" "log detail"; } ol#log { max-height:50vh; } }
@media (max-width: 760px) { main { padding:16px; grid-template-columns: minmax(0,1fr); grid-template-areas: "a" "log" "detail"; }
  header { padding:12px 16px; } .run { margin-left:0; } .run small { text-align:left; } .kpis { grid-template-columns: repeat(2, minmax(0,1fr)); }
  .detail { position:static; max-height:none; } footer { padding:12px 16px; }
  #chart .axis { font-size:12px; } #chart .val { font-size:12.5px; } }
</style>
</head>
<body>
<header>
  <div class="brand"><span class="mark" aria-hidden="true"></span><div><b>Line Inspector</b><small>Candle line 2</small></div></div>
  <div class="mode"><span id="mode" class="badge replay">Replay</span><span id="mode-line"></span><span id="spend" class="spend"></span></div>
  <div class="run"><button id="waiting" type="button" class="pill" hidden></button><span id="progress"></span>
    <button id="start" type="button" class="primary">Start the line</button><small id="start-hint"></small></div>
</header>
<main>
<div class="col-a">
  <section class="panel tray" aria-labelledby="tray-title">
    <h2 id="tray-title">The line</h2>
    <div id="tray-empty" class="empty">Press Start the line; trays appear here.</div>
    <img id="tray" hidden alt="">
    <p id="legend" class="legend" hidden><span><i class="dot pass"></i>passed</span><span><i class="dot reject"></i>rejected</span><span><i class="dot look"></i>second look</span></p>
  </section>
  <section class="panel" aria-labelledby="shift-title"><h2 id="shift-title">This shift</h2><div class="kpis">
    <div><b id="k-trays">0</b><span>trays</span></div><div><b id="k-rejects">0</b><span>rejected</span></div>
    <div><b id="k-looks">0</b><span>second looks</span></div><div><b id="k-waiting">0</b><span>waiting for you</span></div></div></section>
  <section class="panel" aria-labelledby="chart-title"><h2 id="chart-title">Reject rate, every ten trays</h2>
    <svg id="chart" viewBox="0 0 420 190" role="img" aria-label="Reject rate per ten trays: no trays yet"></svg></section>
</div>
<section class="panel log" aria-labelledby="log-title">
  <h2 id="log-title">Reject log</h2>
  <p class="sub">Rejects and second looks, newest first. Parts waiting for you are at the top.</p>
  <div id="log-empty" class="empty">No rejects or second looks yet.</div>
  <ol id="log"></ol>
</section>
<aside id="detail" class="panel detail" aria-label="The chosen part">
  <div id="detail-empty" class="empty">Choose a part in the reject log to see what the line and Line Inspector found, decide it when it waits for you, and ask about it.</div>
  <div id="detail-body" hidden>
    <div class="dhead"><div><h2 id="d-id"></h2><p id="d-sub"></p></div><span id="d-tag" class="tag"></span>
      <button id="d-close" type="button" class="icon" aria-label="Close the part">&times;</button></div>
    <section id="d-found"></section>
    <section id="d-decision"></section>
    <section id="d-thread"></section>
    <p id="d-status" class="status" role="status" hidden></p>
    <form id="ask" hidden>
      <label id="ask-label" for="ask-text">Ask about this part</label>
      <div class="askrow"><textarea id="ask-text" rows="2"></textarea><button id="ask-send" type="submit" class="primary">Send</button></div>
      <p id="ask-note" hidden></p>
    </form>
  </div>
</aside>
</main>
<footer><span>Images: VisA (Zou et al., Amazon), candle class, CC BY 4.0. Scores from SigLIP 2 on this computer against known-good candles.</span><a href="/ui/">Developer view</a></footer>
<script>
const $ = id => document.getElementById(id);
const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const usd = n => '$' + Number(n || 0).toFixed(2);
const hhmm = iso => { const d = new Date(iso); return isNaN(d) ? '' : d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false }); };
const REPLAY = 'A recorded shift played back: no key, no spend. Decisions work; new questions need a live run.';
let S = null, shown = null, chosen = null, P = null, card = null, declining = false, asking = null, notice = null;
const sig = {};
function put(el, html) { if (sig[el.id] !== html) { sig[el.id] = html; el.innerHTML = html; } }
async function call(url, options) {
  const r = await fetch(url, options);
  let body = null;
  try { body = await r.json(); } catch (e) { body = null; }
  return { ok: r.ok, status: r.status, body };
}
function problem(r) { const b = r.body || {}; return (b.error && b.error.message) || b.detail || b.title || (r.status ? 'the app answered ' + r.status : 'the app could not be reached'); }
// Replies may bold a few words in Markdown; everything else stays as written.
function reply(text) { return esc(text).replace(/\*\*([^*\n]+)\*\*/g, '<b>$1</b>'); }
function plain(text) { const t = String(text || 'no reason was given').trim(); return t.length > 300 ? t.slice(0, 300) + '…' : t; }

function header(s) {
  const live = s.mode === 'live';
  const badge = $('mode'); badge.textContent = live ? 'Live' : 'Replay'; badge.className = 'badge ' + s.mode;
  $('mode-line').textContent = live ? 'Claude answers as you watch' : REPLAY;
  $('spend').textContent = live ? `${usd(s.spendUsd)} of ${usd(s.dailyUsd)}` : `${usd(s.spendUsd)} when it was recorded`;
  $('progress').textContent = `${s.trays} of ${s.totalTrays} trays`;
  const words = { waiting: 'Start the line', running: 'Line running', finished: 'Shift finished', stopped: 'Line stopped' };
  const b = $('start'); b.textContent = words[s.phase]; b.disabled = s.phase !== 'waiting';
  $('start-hint').textContent = s.phase === 'finished' ? 'restart the app to run it again'
    : s.phase === 'stopped' ? `${plain(s.stopped)}; restart the app to run it again` : '';
  const w = $('waiting'), n = s.waiting.length;
  w.hidden = n === 0;
  w.textContent = n === 1 ? '1 part waits for your decision' : `${n} parts wait for your decision`;
}

function tray(s) {
  const img = $('tray'), empty = $('tray-empty');
  if (!s.latest) {
    img.hidden = true; empty.hidden = false; $('legend').hidden = true; $('tray-title').textContent = 'The line';
    empty.textContent = s.phase === 'waiting' ? 'Press Start the line; trays appear here.'
      : s.phase === 'running' ? 'Waiting for the first tray: inspecting one takes a few seconds.' : 'No trays came through.';
    return;
  }
  if (s.latestKey === shown) return;
  shown = s.latestKey;
  const id = s.latest, next = new Image();
  next.onload = () => {
    img.src = next.src; img.hidden = false; empty.hidden = true; $('legend').hidden = false;
    img.alt = `Tray ${id}: each candle ringed green when it passed, red when rejected, amber on a second look`;
    $('tray-title').textContent = 'Tray ' + id;
  };
  next.src = `/line/tray/${id}.png?${Date.now()}`;
}

function barPath(x, y, w, h) {
  const r = Math.min(4, w / 2, h);
  return `M${x},${y + h}V${y + r}Q${x},${y} ${x + r},${y}H${x + w - r}Q${x + w},${y} ${x + w},${y + r}V${y + h}Z`;
}
function chart(s) {
  const slots = Math.max(1, Math.ceil(s.totalTrays / 10));
  const W = 420, H = 190, L = 40, R = 8, T = 18, B = 36, pw = W - L - R, ph = H - T - B;
  const step = pw / slots, bw = Math.min(34, step * 0.6);
  let g = '';
  const said = [];
  for (const v of [0, 25, 50, 75, 100]) {
    const y = T + ph - ph * v / 100;
    g += `<line class="grid" x1="${L}" x2="${W - R}" y1="${y}" y2="${y}"/><text class="axis" x="${L - 6}" y="${y + 3.5}" text-anchor="end">${v}%</text>`;
  }
  for (let i = 0; i < slots; i++) {
    const cx = L + step * i + step / 2, first = i * 10 + 1, last = Math.min((i + 1) * 10, s.totalTrays);
    const label = first === last ? `${first}` : `${first}–${last}`;
    g += `<text class="axis" x="${cx}" y="${T + ph + 15}" text-anchor="middle">${label}</text>`;
    const b = s.blocks[i];
    if (!b || !b.candles) continue;
    const r = b.rejected / b.candles, pct = Math.round(r * 100), h = r > 0 ? Math.max(ph * r, 2) : 0, y = T + ph - h;
    said.push(`trays ${label} ${pct}%`);
    g += `<g><title>Trays ${label}: ${b.rejected} of ${b.candles} candles rejected (${pct}%)</title>`
      + (h > 0 ? `<path class="bar" d="${barPath(cx - bw / 2, y, bw, h)}"/>` : '')
      + `<text class="val" x="${cx}" y="${y - 5}" text-anchor="middle">${pct}%</text>`
      + `<rect class="hit" x="${cx - step / 2}" y="${T}" width="${step}" height="${ph}"/></g>`;
  }
  g += `<line class="base" x1="${L}" x2="${W - R}" y1="${T + ph}" y2="${T + ph}"/>`
    + `<text class="axis" x="${L + pw / 2}" y="${H - 4}" text-anchor="middle">trays</text>`;
  const svg = $('chart');
  put(svg, g);
  svg.setAttribute('aria-label', 'Reject rate per ten trays: ' + (said.join(', ') || 'no trays yet'));
}

function log(s) {
  const rows = [...s.log].sort((a, b) => Number(s.waiting.includes(b.id)) - Number(s.waiting.includes(a.id)));
  $('log-empty').hidden = rows.length > 0;
  put($('log'), rows.map(p => `<li><button type="button" class="row${p.id === chosen ? ' on' : ''}" data-id="${esc(p.id)}" aria-pressed="${p.id === chosen}">`
    + `<span class="tag ${esc(p.tone)}">${esc(p.tag)}</span><b>${esc(p.id)}</b>`
    + `<span class="meta">score ${p.score.toFixed(2)}${p.mm != null ? ' · ' + p.mm.toFixed(1) + ' mm' : ''} · ${esc(p.said)}</span></button></li>`).join(''));
}

async function tick() {
  let r;
  try { r = await call('/line/state'); } catch (e) { return; }
  if (!r.ok || !r.body) return;
  S = r.body;
  header(S); tray(S);
  $('k-trays').textContent = S.trays; $('k-rejects').textContent = S.rejects;
  $('k-looks').textContent = S.looks; $('k-waiting').textContent = S.waiting.length;
  chart(S); log(S);
}

function choose(id) {
  if (chosen !== id) {
    chosen = id; P = null; card = null; declining = false; notice = null;
    for (const k of ['d-found', 'd-decision', 'd-thread']) { delete sig[k]; $(k).innerHTML = ''; }
    $('d-status').hidden = true; $('ask').hidden = true; $('ask-text').value = '';
    $('d-id').textContent = id; $('d-sub').textContent = ''; $('d-tag').textContent = ''; $('d-tag').className = 'tag';
  }
  $('detail-empty').hidden = true; $('detail-body').hidden = false;
  if (S) log(S);
  refresh();
  if (window.matchMedia('(max-width: 760px)').matches) $('detail').scrollIntoView({ behavior: 'smooth', block: 'start' });
}

let busy = false, again = false;
async function refresh() {
  if (!chosen) return;
  if (busy) { again = true; return; }
  busy = true;
  const id = chosen;
  try {
    const r = await call('/line/part/' + encodeURIComponent(id));
    if (!r.ok || !r.body || chosen !== id) return;
    const p = r.body;
    let open = null, entries = [];
    if (p.hasSession) {
      const q = encodeURIComponent(id);
      const [d, t] = await Promise.all([
        call(`/api/surface/decisions?surface=web&session=${q}&decidedSince=2000-01-01T00:00:00Z`),
        call(`/api/surface/sessions/${q}/thread?surface=web`),
      ]);
      if (chosen !== id) return;
      if (d.ok && d.body) {
        const mine = (d.body.decisions || []).filter(c => c.sessionId === id);
        open = mine.find(c => c.status === 'waiting') || mine[0] || null;
      }
      if (t.ok && t.body) entries = t.body.entries || [];
    }
    P = p;
    render(p, open, entries);
  } catch (e) {
    // The next poll tries again.
  } finally {
    busy = false;
    if (again) { again = false; refresh(); }
  }
}

function render(p, c, entries) {
  $('d-id').textContent = p.id;
  $('d-sub').textContent = `Tray ${p.tray} · candle ${p.candle} · ${p.said}`;
  const tag = $('d-tag'); tag.textContent = p.tag; tag.className = 'tag ' + p.tone;
  found(p, c);
  decision(p, c);
  conversation(p, entries);
  status(p);
  askBox(p);
}

function band(score) {
  return score >= 0.75 ? 'above reject (0.75)' : score < 0.45 ? 'below pass (0.45)' : 'between pass (0.45) and reject (0.75)';
}

function found(p, c) {
  const rows = [['Score on this computer', `${p.score.toFixed(2)}, ${band(p.score)}`]];
  if (p.longestMm != null) rows.push(['Defect measured', `${p.longestMm.toFixed(1)} mm across, ${Math.round(p.areaMm2 ?? 0)} mm²`]);
  if (p.stage === 'borderline') {
    let v;
    if (p.agent) v = `${p.agent.verdict === 'reject' ? 'Rejected' : 'Passed'}: ${p.agent.reason || ''}`;
    else if (p.decision) v = p.decision.answer ? 'The two models disagreed, so the part was yours to decide.' : 'The two models disagree, so the part is yours to decide.';
    else if (p.turn && p.turn.state === 'failed') v = 'The second look stopped before a verdict.';
    else v = 'Taking a second look.';
    rows.push(['Line Inspector', v]);
  } else {
    rows.push(['Line Inspector', 'Not involved: the line decides parts outside the borderline band on its own.']);
  }
  rows.push(["VisA's label, for scoring", p.defective ? String(p.defect || 'a defect').replaceAll('_', ' ') : 'no defect']);
  const legend = p.longestMm != null ? 'Its least familiar area on this computer boxed in amber; the measured defect in red.' : 'Its least familiar area on this computer boxed in amber.';
  const crop = c && c.status === 'waiting' ? ''
    : `<figure><img src="/line/part/${encodeURIComponent(p.id)}/crop.png" width="360" alt="Candle ${esc(p.id)}. ${legend}"><figcaption>${legend}</figcaption></figure>`;
  put($('d-found'), `<h3>What the line found</h3>${crop}<dl>${rows.map(([k, v]) => `<dt>${esc(k)}</dt><dd>${esc(v)}</dd>`).join('')}</dl>`);
}

function decision(p, c) {
  card = c;
  const el = $('d-decision');
  if (!c) { put(el, ''); return; }
  const a = (p.decision && p.decision.args) || (c.call && c.call.args) || {};
  const suggested = a.suggestion === 'pass' ? 'pass' : 'reject';
  if (c.status === 'waiting') {
    const m = (S && S.models) || {};
    put(el, `<h3>Your decision</h3><div class="card-d">`
      + `<p class="q">${esc(c.heading)}</p>`
      + `<p class="proposal">Line Inspector suggests: <span class="tag ${suggested}">${suggested === 'pass' ? 'Pass the part' : 'Reject the part'}</span></p>`
      + `<figure><img src="/line/part/${encodeURIComponent(p.id)}/crop.png" width="240" alt="Candle ${esc(p.id)}, its least familiar area on this computer boxed in amber"><figcaption>The candle the models looked at; its least familiar area on this computer boxed in amber.</figcaption></figure>`
      + `<dl><dt>Reasoner · ${esc(m.reasoner)}</dt><dd>${esc(a.reasoner)}</dd>`
      + `<dt>Second opinion · ${esc(m.second)}</dt><dd>${esc(a.verifier)}</dd>`
      + `<dt>Measured</dt><dd>${esc(a.measured || 'not measured')}</dd>`
      + `<dt>Score on this computer</dt><dd>${p.score.toFixed(2)}, ${band(p.score)}</dd></dl>`
      + `<div class="actions"${declining ? ' hidden' : ''}><button type="button" class="primary" data-act="approve">Approve</button><button type="button" class="secondary" data-act="decline">Decline</button></div>`
      + `<div class="declining"${declining ? '' : ' hidden'}><label for="reason">Your reason, for Line Inspector (optional)</label><textarea id="reason" rows="2"></textarea>`
      + `<div class="actions" style="margin-top:8px"><button type="button" class="danger" data-act="confirm">Confirm decline</button><button type="button" class="link" data-act="cancel">Cancel</button></div></div>`
      + `<p id="d-err" class="err" hidden></p>`
      + `<p class="fine">Approve ${suggested === 'pass' ? 'passes' : 'rejects'} the part, as Line Inspector suggests. Decline ${suggested === 'pass' ? 'rejects' : 'passes'} it instead, and your reason goes to Line Inspector.</p></div>`);
    return;
  }
  const d = c.decided || {};
  const at = d.at ? hhmm(d.at) : '';
  const answer = (p.decision && p.decision.answer) || (d.answer === 'yes' ? 'approved' : d.answer === 'no' ? 'declined' : null);
  const reason = p.decision && p.decision.reason;
  const line = answer === 'approved' ? `You approved at ${at}`
    : answer === 'declined' ? `You declined at ${at}${reason ? ': ' + reason : ''}`
    : `This decision ended at ${at} without an answer`;
  put(el, `<h3>Your decision</h3><div class="card-d done"><p style="margin:0"><b>${esc(line)}</b></p>`
    + `<p class="fine">${esc(c.heading)} Line Inspector suggested: ${suggested} the part.</p></div>`);
}

function showErr(text) { const e = $('d-err'); if (e) { e.textContent = text; e.hidden = false; } }

async function answerCard(c, answer, reason) {
  const command = { type: 'decide', decisionId: c.id, version: c.version, answer, expanded: true };
  if (answer === 'deny') { if (reason) command.reason = reason; }
  else command.evidence = ((c.media && c.media.images) || []).map(i => i.digest);
  let r;
  try {
    r = await call('/api/surface/commands', { method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ surface: 'web', origin: { kind: 'pointer' }, command }) });
  } catch (e) { r = { ok: false, status: 0, body: null }; }
  const kind = r.ok && r.body ? r.body.type : null;
  delete sig['d-decision'];
  if (kind === 'answered' || kind === 'alreadyDecided' || kind === 'pending') { declining = false; await refresh(); return; }
  if (kind === 'changed' || r.status === 409) {
    await refresh();
    showErr('The decision changed while you looked at it: check it again, then answer.');
    return;
  }
  decision(P, card);
  showErr('Your answer was not recorded: ' + problem(r));
}

function conversation(p, entries) {
  const el = $('d-thread');
  if (!p.hasSession) {
    put(el, p.stage === 'borderline' ? '' : '<h3>Conversation</h3><p class="fine">The line decided this part on its own, so there is no conversation about it.</p>');
    return;
  }
  const turns = [];
  let cur = null;
  for (const e of entries) {
    for (const it of e.items || []) {
      if (it.kind === 'message' && it.from === 'you') { cur = { ask: it.text, at0: e.at, at1: e.at, steps: [], tail: [], waited: 0, held: null }; turns.push(cur); continue; }
      if (!cur) { cur = { ask: null, at0: e.at, at1: e.at, steps: [], tail: [], waited: 0, held: null }; turns.push(cur); }
      cur.at1 = e.at;
      // Time spent waiting for the engineer is theirs, not the agent's work.
      if (cur.held && it.kind !== 'decision') { cur.waited += new Date(e.at) - new Date(cur.held); cur.held = null; }
      if (it.kind === 'decision') cur.held = e.at;
      // Only the text after a turn's last tool step is its reply; text between steps is the agent's own note.
      if (it.kind === 'message') cur.tail.push(it.text);
      else if (it.kind === 'activity') { cur.steps.push(it.text); cur.tail = []; }
      else if (it.kind === 'result' || it.kind === 'run' || it.kind === 'decision') cur.tail = [];
    }
  }
  const html = turns.map(t => {
    let h = '';
    if (t.ask != null) {
      h += /^Borderline part /.test(t.ask)
        ? `<p class="sys">The line sent ${esc(p.id)} for a second look at ${hhmm(t.at0)}.</p>`
        : `<div class="msg you"><div class="who">You · ${hhmm(t.at0)}</div><p>${esc(t.ask)}</p></div>`;
    }
    if (t.steps.length) {
      const secs = Math.max(1, Math.round((new Date(t.at1) - new Date(t.at0) - t.waited) / 1000));
      const waited = Math.round(t.waited / 1000);
      h += `<details class="steps"><summary>Worked for ${secs} s · ${t.steps.length} ${t.steps.length === 1 ? 'step' : 'steps'}${waited > 0 ? ` · waited ${waited} s for you` : ''}</summary>`
        + `<ol>${t.steps.map(x => `<li>${esc(x)}</li>`).join('')}</ol></details>`;
    }
    const last = t.tail[t.tail.length - 1];
    if (last) h += `<div class="msg agent"><div class="who">Line Inspector</div><p>${reply(last)}</p></div>`;
    return h;
  }).join('');
  put(el, `<h3>Conversation about ${esc(p.id)}</h3>${html || '<p class="fine">Nothing yet.</p>'}`);
}

function status(p) {
  const el = $('d-status'), t = p.turn;
  if (asking && asking.id === p.id && t && t.asked && t.state !== 'running' && Date.parse(t.at) >= asking.since) asking = null;
  if (asking && Date.now() - asking.since > 180000) asking = null;
  if (notice && notice.id === p.id && t && t.asked && t.state === 'running') notice = null;
  let text = '', fail = false;
  if (t && t.state === 'running') {
    text = t.asked ? 'Line Inspector is answering…'
      : p.decision && p.decision.answer ? 'Line Inspector is finishing the second look…' : 'Line Inspector is taking a second look…';
  }
  else if (asking && asking.id === p.id) text = 'Line Inspector is answering…';
  else if (t && t.state === 'failed') {
    fail = true;
    text = (t.asked ? 'Line Inspector could not answer: ' : 'The second look failed: ') + plain(t.error);
  }
  if (!text && notice && notice.id === p.id) text = notice.text;
  el.textContent = text; el.className = 'status' + (fail ? ' fail' : ''); el.hidden = !text;
}

function askBox(p) {
  const f = $('ask');
  f.hidden = !p.hasSession;
  if (!p.hasSession) return;
  const live = !!S && S.mode === 'live', ta = $('ask-text'), note = $('ask-note');
  $('ask-label').textContent = `Ask about ${p.id}`;
  ta.placeholder = `Ask about ${p.id}…`;
  ta.disabled = !live;
  $('ask-send').disabled = !live || !!(asking && asking.id === p.id);
  note.hidden = live;
  if (!live) note.innerHTML = 'Questions need a live run: set <code>ANTHROPIC_API_KEY</code> and start with <code>serve --live</code>.';
}

$('start').addEventListener('click', async () => {
  $('start').disabled = true;
  try { await call('/line/start', { method: 'POST' }); } catch (e) { /* the next poll shows the phase */ }
  tick();
});
$('waiting').addEventListener('click', () => { if (S && S.waiting.length) choose(S.waiting.find(id => id !== chosen) || S.waiting[0]); });
$('log').addEventListener('click', e => { const b = e.target.closest('button[data-id]'); if (b) choose(b.dataset.id); });
$('d-close').addEventListener('click', () => {
  chosen = null; P = null; card = null;
  $('detail-body').hidden = true; $('detail-empty').hidden = false;
  if (S) log(S);
});
$('d-decision').addEventListener('click', async e => {
  const b = e.target.closest('button[data-act]');
  if (!b || !card || !P) return;
  const act = b.dataset.act;
  if (act === 'decline') { declining = true; decision(P, card); $('reason').focus(); return; }
  if (act === 'cancel') { declining = false; decision(P, card); return; }
  const reason = act === 'confirm' ? $('reason').value.trim() : '';
  for (const x of $('d-decision').querySelectorAll('button')) x.disabled = true;
  await answerCard(card, act === 'approve' ? 'allowOnce' : 'deny', reason);
});
$('ask').addEventListener('submit', async e => {
  e.preventDefault();
  const id = chosen, text = $('ask-text').value.trim();
  if (!id || !text || !S || S.mode !== 'live') return;
  $('ask-send').disabled = true;
  let r;
  try {
    r = await call('/api/surface/commands', { method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ surface: 'web', origin: { kind: 'keyboard' }, command: { type: 'send', sessionId: id, text } }) });
  } catch (err) { r = { ok: false, status: 0, body: null }; }
  if (r.ok && r.body) {
    $('ask-text').value = '';
    if (r.body.type === 'queued') notice = { id, text: `${r.body.text}.` };
    else asking = { id, since: Date.now() - 2000 };
  } else {
    notice = { id, text: 'Your question was not sent: ' + problem(r) };
    $('ask-send').disabled = false;
  }
  refresh();
});
$('ask-text').addEventListener('keydown', e => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); $('ask').requestSubmit(); } });
tick();
setInterval(tick, 1000);
setInterval(refresh, 1500);
</script>
</body>
</html>
"##;
