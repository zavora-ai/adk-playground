//! The web home while the line runs at one tray a second: each second look as a thread, the
//! parts the models disagree on on the engineer's desk, and the line page with the latest tray,
//! the reject log and the reject rate.

use std::future::IntoFuture;
use std::sync::Arc;
use std::time::Duration;

use adk_artifact::ArtifactService;
use adk_core::{RequestContext, SingleAgentLoader};
use adk_server::auth_bridge::{RequestContextError, RequestContextExtractor};
use adk_server::{ServerBuilder, ServerConfig};
use adk_surface::{CardHint, CardHints, LocalSource, Place, SurfaceEngine};
use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use serde_json::json;

use crate::agent::{APP, USER};
use crate::line::TrayFeed;
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

pub async fn serve(common: Common, port: u16) -> anyhow::Result<()> {
    let app = Arc::new(App::build(&common).await?);
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
    let page = axum::Router::new()
        .route("/line", get(|| async { Html(PAGE) }))
        .route("/line/state", get(state))
        .route("/line/tray/{id}", get(tray_image))
        .with_state(Arc::clone(&app));
    let router = ServerBuilder::new(config)
        .add_api_routes(adk_surface::web::router(engine))
        .add_root_routes(adk_surface::web::home().merge(page))
        .build();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("Line Inspector on http://127.0.0.1:{port}/line");
    println!("the engineer's desk and each second look on http://127.0.0.1:{port}/ui/");
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
        let app = Arc::clone(&app);
        tokio::spawn(async move {
            match crate::line::run(
                TrayFeed::new(app.trays.clone(), true),
                Arc::clone(&app.inspector),
                Arc::clone(&app.line),
            )
            .await
            {
                Ok(_) => println!(
                    "[line] the shift's trays are through; the desk stays open until Ctrl-C"
                ),
                Err(err) => eprintln!("[line] stopped: {err}"),
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

/// A tray with each candle ringed: green passed, red rejected, amber on a second look.
async fn tray_image(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
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
    let mut verdicts = Vec::new();
    for part in &parts {
        verdicts.push((part.candle, crate::verdict(&app, part).await));
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

async fn state(State(app): State<Arc<App>>) -> impl IntoResponse {
    let seen = app
        .book
        .trays_seen
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let parts = app.book.all();
    let (mut rejects, mut looks, mut engineer) = (0, 0, 0);
    let mut blocks: Vec<(usize, usize)> = Vec::new();
    let mut recent = Vec::new();
    for part in &parts {
        let v = crate::verdict(&app, part).await;
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
        if v == "engineer" {
            engineer += 1;
        }
        if part.stage != "pass" {
            recent.push(json!({ "id": part.id, "score": part.score, "stage": part.stage, "verdict": v, "mm": part.longest_mm, "defect": part.defect }));
        }
    }
    recent.reverse();
    recent.truncate(40);
    let spend: f64 = app.tape.spend().values().map(|s| s.usd).sum();
    axum::Json(json!({
        "latest": seen.last(),
        "trays": seen.len(),
        "parts": parts.len(),
        "rejects": rejects,
        "looks": looks,
        "engineer": engineer,
        "rate": blocks.iter().map(|(r, n)| if *n == 0 { 0.0 } else { *r as f64 / *n as f64 }).collect::<Vec<_>>(),
        "recent": recent,
        "spendUsd": (spend * 10_000.0).round() / 10_000.0,
        "spendLabel": if app.tape.mode == crate::tape::Mode::Replay { "the recorded live run's spend" } else { "model spend" },
    }))
}

const PAGE: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="icon" href="data:,">
<title>Line Inspector</title>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap">
<style>
:root { --bg:#fafaf9; --card:#fff; --border:#e7e6e2; --text:#18181b; --text2:#4b4b53; --label:#6b6b74; --btn:#c2410c; --btnText:#fff;
  --alert:#b42318; --alertBg:#fdecea; --ok:#1e6b45; --okBg:#e7f4ec; --warn:#8a4b00; --warnBg:#fdf3e1; --bar:#c2410c; }
@media (prefers-color-scheme: dark) { :root { --bg:#0e0f11; --card:#16171a; --border:#26272c; --text:#ededef; --text2:#b5b5bd; --label:#8e8e97;
  --btn:#ff6a1f; --btnText:#0e0f11; --alert:#ff8a80; --alertBg:#3a1d1b; --ok:#7ad3a0; --okBg:#17301f; --warn:#f5c26b; --warnBg:#3a2c12; --bar:#ff6a1f; } }
* { box-sizing: border-box; }
body { margin:0; font-family: Inter, system-ui, sans-serif; background: var(--bg); color: var(--text); }
header { display:flex; flex-wrap:wrap; align-items:center; gap:12px 20px; padding:14px 28px; border-bottom:1px solid var(--border); background:var(--card); }
.logo { display:flex; align-items:center; gap:10px; font-weight:700; }
.logo span { width:28px; height:28px; border-radius:7px; background:#f74c00; display:inline-block; }
header a.desk { margin-left:auto; font-weight:600; font-size:14px; padding:10px 16px; border-radius:8px; background:var(--btn); color:var(--btnText); text-decoration:none; }
main { max-width:1400px; margin:0 auto; padding:24px 28px; display:flex; flex-wrap:wrap; gap:24px; align-items:flex-start; }
.panel { background:var(--card); border:1px solid var(--border); border-radius:12px; padding:16px; }
.panel h2 { margin:0 0 10px; font-size:15px; }
.tray { flex: 2 1 520px; min-width:0; }
.tray img { width:100%; border-radius:8px; display:block; background:#000; aspect-ratio: 642 / 584; }
aside { flex: 1 1 360px; min-width:0; display:flex; flex-direction:column; gap:16px; }
.kpis { display:grid; grid-template-columns: repeat(4, minmax(0,1fr)); gap:10px; }
.kpis b { display:block; font-size:22px; } .kpis span { font-size:12px; color:var(--label); }
svg { width:100%; height:120px; }
ol { list-style:none; margin:0; padding:0; max-height:46vh; overflow:auto; font-size:13px; }
li { border-top:1px solid var(--border); padding:8px 0; display:flex; gap:8px; align-items:center; flex-wrap:wrap; }
.tag { font-size:11px; font-weight:700; padding:3px 8px; border-radius:999px; }
.reject { background:var(--alertBg); color:var(--alert); } .pass { background:var(--okBg); color:var(--ok); } .engineer, .undecided { background:var(--warnBg); color:var(--warn); }
footer { padding:12px 28px; font-size:11px; color:var(--label); border-top:1px solid var(--border); }
@media (max-width: 720px) { main { padding:16px; } header { padding:14px 16px; } .kpis { grid-template-columns: repeat(2, minmax(0,1fr)); } }
</style>
</head>
<body>
<header><div class="logo"><span aria-hidden="true"></span>Line Inspector</div><span style="color:var(--text2);font-size:13px">Candle line 2</span>
<a class="desk" href="/ui/">Engineer's desk</a></header>
<main>
<section class="panel tray"><h2 id="tray-title">Waiting for the first tray</h2><img id="tray" alt="The latest tray, each candle ringed green when it passed, red when rejected, amber on a second look"></section>
<aside>
<div class="panel"><h2>This shift</h2><div class="kpis">
<div><b id="trays">0</b><span>trays</span></div><div><b id="rejects">0</b><span>rejected</span></div>
<div><b id="looks">0</b><span>second looks</span></div><div><b id="engineer">0</b><span>with the engineer</span></div></div>
<p style="margin:10px 0 0;font-size:12px;color:var(--label)"><span id="spend">$0</span> <span id="spend-label">model spend</span></p></div>
<div class="panel"><h2>Reject rate, every ten trays</h2><svg id="chart" role="img" aria-label="Reject rate per block of ten trays"></svg></div>
<div class="panel"><h2>Reject log</h2><ol id="log"></ol></div>
</aside>
</main>
<footer>Images: VisA (Zou et al., Amazon), candle class, CC BY 4.0. Scores from SigLIP 2 on this computer against known-good candles.</footer>
<script>
function esc(s) { return String(s ?? '').replace(/[&<>"]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c])); }
let shown = null;
async function tick() {
  try {
    const s = await (await fetch('/line/state')).json();
    for (const k of ['trays', 'rejects', 'looks', 'engineer']) document.getElementById(k).textContent = s[k];
    document.getElementById('spend').textContent = '$' + s.spendUsd.toFixed(2);
    document.getElementById('spend-label').textContent = s.spendLabel;
    if (s.latest && s.latest !== shown) { shown = s.latest; document.getElementById('tray').src = '/line/tray/' + s.latest + '.png?' + Date.now(); document.getElementById('tray-title').textContent = 'Tray ' + s.latest; }
    const w = 100 / Math.max(s.rate.length, 1);
    document.getElementById('chart').innerHTML = s.rate.map((r, i) => `<rect x="${i * w + 1}%" width="${w - 2}%" y="${100 - Math.min(r * 400, 100)}%" height="${Math.min(r * 400, 100)}%" fill="var(--bar)"><title>Trays ${i * 10 + 1}–${i * 10 + 10}: ${(r * 100).toFixed(0)}% of candles rejected</title></rect>`).join('');
    document.getElementById('log').innerHTML = s.recent.map(p => `<li><span class="tag ${esc(p.verdict)}">${esc(p.verdict)}</span><b>${esc(p.id)}</b><span>score ${p.score.toFixed(2)}${p.mm != null ? ' · ' + p.mm.toFixed(1) + ' mm' : ''}${p.stage === 'borderline' ? ' · second look' : ''}</span></li>`).join('');
  } catch (e) {}
}
tick(); setInterval(tick, 1000);
</script>
</body>
</html>
"##;
