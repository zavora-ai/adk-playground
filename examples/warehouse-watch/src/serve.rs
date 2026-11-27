//! The web home over the shift: the four cameras played at their own speed on the
//! Warehouse Watch page, each incident's review as a thread, and the near-misses the agent
//! raises held on the supervisor's desk with the frame as evidence.

use std::future::IntoFuture;
use std::sync::Arc;
use std::time::{Duration, Instant};

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

use crate::agent::Desk;
use crate::watch::{APP, Camera, Incidents, USER, clock};
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

/// The heading an incident card shows on the supervisor's desk.
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

#[derive(Clone)]
struct Page {
    cameras: Vec<Arc<Camera>>,
    incidents: Arc<Incidents>,
    desk: Arc<Desk>,
    out: std::path::PathBuf,
    tape: Arc<crate::tape::Tape>,
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
    let page = Page {
        cameras: cameras.clone(),
        incidents: Arc::clone(&incidents),
        desk: Arc::clone(&desk),
        out: common.out.clone(),
        tape: Arc::clone(&tape),
    };
    let warehouse = axum::Router::new()
        .route("/warehouse", get(|| async { Html(PAGE) }))
        .route("/warehouse/state", get(state))
        .route("/warehouse/camera/{name}", get(picture))
        .with_state(page);
    let app = ServerBuilder::new(config)
        .add_api_routes(adk_surface::web::router(engine))
        .add_root_routes(adk_surface::web::home().merge(warehouse))
        .build();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("Warehouse Watch on http://127.0.0.1:{port}/warehouse");
    println!("the supervisor's desk and each incident's thread on http://127.0.0.1:{port}/ui/");
    let stopping = Arc::new(tokio::sync::Notify::new());
    let server = tokio::spawn(
        axum::serve(listener, app)
            .with_graceful_shutdown({
                let stopping = Arc::clone(&stopping);
                async move { stopping.notified().await }
            })
            .into_future(),
    );

    // The shift plays once at the video's own speed, every frame; the page and the desk stay
    // up after it.
    let root = crate::root();
    let seconds = common.seconds;
    let plays = tokio::spawn(async move {
        let started = Instant::now();
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
        println!("[shift] the video has ended; the desk stays open until Ctrl-C");
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

async fn picture(State(page): State<Page>, Path(name): Path<String>) -> Response {
    let name = name.trim_end_matches(".png");
    let Some(camera) = page.cameras.iter().find(|c| c.name == name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::task::spawn_blocking({
        let camera = Arc::clone(camera);
        move || camera.picture()
    })
    .await
    {
        Ok(Some((_, png))) => (
            [
                (header::CONTENT_TYPE, "image/png"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            png,
        )
            .into_response(),
        _ => StatusCode::NO_CONTENT.into_response(),
    }
}

async fn state(State(page): State<Page>) -> impl IntoResponse {
    let mut rows = Vec::new();
    let (mut raised, mut logged) = (0, 0);
    for incident in page.incidents.all().into_iter().rev() {
        let decided = page.desk.decision(&incident.id).await;
        let signed = page
            .out
            .join("incidents")
            .join(format!("{}.json", incident.id))
            .exists();
        let status = match decided.as_deref() {
            Some("raise_incident") if signed => "Signed off",
            Some("raise_incident") => "Waiting for sign-off",
            Some(_) => "Logged, not a near-miss",
            None => "Reviewing",
        };
        match decided.as_deref() {
            Some("raise_incident") => raised += 1,
            Some(_) => logged += 1,
            None => {}
        }
        rows.push(json!({
            "id": incident.id,
            "camera": incident.camera,
            "at": clock(incident.at_ms),
            "gapM": incident.gap_m,
            "vehicle": incident.vehicle,
            "speed": incident.vehicle_speed_m_s,
            "status": status,
        }));
    }
    let cameras: Vec<_> = page
        .cameras
        .iter()
        .map(|c| json!({ "name": c.name, "spots": c.spots() }))
        .collect();
    let spend: f64 = page.tape.spend().values().map(|s| s.usd).sum();
    axum::Json(json!({
        "cameras": cameras,
        "incidents": rows,
        "raised": raised,
        "logged": logged,
        "spendUsd": (spend * 10_000.0).round() / 10_000.0,
        "spendLabel": if page.tape.mode == crate::tape::Mode::Replay {
            "the recorded live run's spend"
        } else {
            "model spend"
        },
        "source": format!("{}@{} {}", data::REPO, &data::REVISION[..10], data::SCENE),
    }))
}

const PAGE: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="icon" href="data:,">
<title>Warehouse Watch</title>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap">
<style>
:root { --bg:#fafaf9; --card:#fff; --border:#e7e6e2; --text:#18181b; --text2:#4b4b53; --label:#6b6b74;
  --btn:#c2410c; --btnText:#fff; --alert:#b42318; --alertBg:#fdecea; --ok:#1e6b45; --okBg:#e7f4ec; --warn:#8a4b00; --warnBg:#fdf3e1; }
@media (prefers-color-scheme: dark) { :root { --bg:#0e0f11; --card:#16171a; --border:#26272c; --text:#ededef; --text2:#b5b5bd;
  --label:#8e8e97; --btn:#ff6a1f; --btnText:#0e0f11; --alert:#ff8a80; --alertBg:#3a1d1b; --ok:#7ad3a0; --okBg:#17301f; --warn:#f5c26b; --warnBg:#3a2c12; } }
* { box-sizing: border-box; }
body { margin:0; font-family: Inter, system-ui, sans-serif; background: var(--bg); color: var(--text); }
header { display:flex; flex-wrap:wrap; align-items:center; gap:12px 20px; padding:14px 28px; border-bottom:1px solid var(--border); background:var(--card); }
.logo { display:flex; align-items:center; gap:10px; font-weight:700; }
.logo span { width:28px; height:28px; border-radius:7px; background:#f74c00; display:inline-block; }
header a.desk { margin-left:auto; font-weight:600; font-size:14px; padding:10px 16px; border-radius:8px; background:var(--btn); color:var(--btnText); text-decoration:none; }
main { max-width:1400px; margin:0 auto; padding:24px 28px; display:flex; flex-wrap:wrap; gap:24px; align-items:flex-start; }
.cams { flex: 999 1 640px; min-width:0; display:grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap:16px; }
@media (max-width: 720px) { .cams { grid-template-columns: minmax(0, 1fr); } main { padding:16px; } header { padding:14px 16px; } }
.cam, .panel { background:var(--card); border:1px solid var(--border); border-radius:12px; overflow:hidden; }
.cam h2 { margin:0; padding:10px 14px; font-size:14px; display:flex; justify-content:space-between; }
.cam h2 small { color:var(--label); font-weight:500; }
.cam img { display:block; width:100%; aspect-ratio:16/9; background:#000; object-fit:cover; }
aside { flex: 1 1 360px; min-width:0; display:flex; flex-direction:column; gap:16px; }
.panel { padding:16px; }
.panel h2 { margin:0 0 10px; font-size:15px; }
.kpis { display:grid; grid-template-columns:repeat(3, 1fr); gap:10px; }
.kpi b { display:block; font-size:22px; }
.kpi span { font-size:12px; color:var(--label); }
ol { list-style:none; margin:0; padding:0; display:flex; flex-direction:column; gap:8px; }
li { border-top:1px solid var(--border); padding-top:8px; font-size:13px; display:flex; flex-direction:column; gap:3px; }
li .tag { align-self:flex-start; font-size:11px; font-weight:700; padding:3px 8px; border-radius:999px; }
.Waiting { background:var(--alertBg); color:var(--alert); } .Signed { background:var(--okBg); color:var(--ok); }
.Logged { background:var(--warnBg); color:var(--warn); } .Reviewing { background:var(--border); color:var(--text2); }
.legend { font-size:12px; color:var(--text2); }
footer { padding:12px 28px; font-size:11px; color:var(--label); border-top:1px solid var(--border); }
</style>
</head>
<body>
<header><div class="logo"><span aria-hidden="true"></span>Warehouse Watch</div>
<div class="legend">People found on this computer <b style="color:#22c55e">■</b> · vehicles Claude spotted <b style="color:#ff6a1f">■</b></div>
<a class="desk" href="/ui/">Supervisor's desk</a></header>
<main>
<section class="cams" id="cams" aria-label="Cameras"></section>
<aside>
<div class="panel"><h2>This shift</h2><div class="kpis">
<div class="kpi"><b id="raised">0</b><span>near-misses raised</span></div>
<div class="kpi"><b id="logged">0</b><span>approaches dismissed</span></div>
<div class="kpi"><b id="spend">$0</b><span id="spend-label">model spend</span></div></div></div>
<div class="panel"><h2>Close approaches</h2><ol id="list"><li>None yet. The agent sleeps until a vehicle and a person come within 1.5 m.</li></ol></div>
</aside>
</main>
<footer id="foot">Video and ground truth: NVIDIA PhysicalAI-SmartSpaces, CC BY 4.0 (synthetic; no real people).</footer>
<script>
const cams = document.getElementById('cams');
function esc(s) { return String(s).replace(/[&<>"]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c])); }
async function tick() {
  try {
    const s = await (await fetch('/warehouse/state')).json();
    if (!cams.children.length) {
      cams.innerHTML = s.cameras.map(c => `<figure class="cam" style="margin:0"><h2>${esc(c.name.replace('_',' '))}<small id="t-${c.name}"></small></h2><img id="i-${c.name}" alt="${esc(c.name)}, latest frame"></figure>`).join('');
    }
    for (const c of s.cameras) {
      if (c.spots > 0) document.getElementById('i-' + c.name).src = '/warehouse/camera/' + c.name + '.png?' + Date.now();
      document.getElementById('t-' + c.name).textContent = c.spots + ' spots';
    }
    document.getElementById('raised').textContent = s.raised;
    document.getElementById('logged').textContent = s.logged;
    document.getElementById('spend').textContent = '$' + s.spendUsd.toFixed(2);
    document.getElementById('spend-label').textContent = s.spendLabel;
    if (s.incidents.length) {
      document.getElementById('list').innerHTML = s.incidents.map(i => `<li><span class="tag ${i.status.split(' ')[0].replace(',','')}">${esc(i.status)}</span><b>${esc(i.id)} · ${esc(i.camera.replace('_',' '))} at ${esc(i.at)}</b><span>A person and a ${esc(i.vehicle)} ${i.gapM.toFixed(1)} m apart, the vehicle at ${i.speed.toFixed(1)} m/s</span></li>`).join('');
    }
    document.getElementById('foot').textContent = 'Video and ground truth: ' + s.source + ', NVIDIA, CC BY 4.0 (synthetic; no real people).';
  } catch (e) {}
}
tick(); setInterval(tick, 700);
</script>
</body>
</html>
"##;
