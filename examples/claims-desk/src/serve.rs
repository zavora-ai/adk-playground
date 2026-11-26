//! The web home while the day's claims are worked: each claim's assessment as a thread, the
//! estimates and referrals on the adjuster's desk with the photos, and the Claims Desk page.

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

use crate::desk::{APP, USER};
use crate::{App, Common, data};

struct Adjuster;

#[async_trait]
impl RequestContextExtractor for Adjuster {
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

struct ClaimCards;

#[async_trait]
impl CardHints for ClaimCards {
    async fn hint(&self, request: &adk_core::ApprovalRequest) -> Option<CardHint> {
        matches!(request.tool_name.as_str(), "draft_estimate" | "refer_claim").then(|| {
            let claim = request.args_redacted["claim"].as_str().unwrap_or_default();
            let title = request.args_redacted["title"]
                .as_str()
                .unwrap_or("A claim to decide");
            CardHint::new().with_heading(format!("{claim}: {title}"))
        })
    }
}

pub async fn serve(common: Common, port: u16) -> anyhow::Result<()> {
    let app = Arc::new(App::build(&common).await?);
    let desk = Arc::clone(&app.desk);
    desk.kernel.start().await?;
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
        .agent_name(APP, "Claims Desk")
        .card_hints(Arc::new(ClaimCards))
        .build();
    let config = ServerConfig::new(
        Arc::new(SingleAgentLoader::new(desk.agent.clone())),
        desk.sessions.clone(),
    )
    .with_request_context(Arc::new(Adjuster));
    let page = axum::Router::new()
        .route("/claims", get(|| async { Html(PAGE) }))
        .route("/claims/state", get(state))
        .route("/claims/photo/{name}", get(photo))
        .with_state(Arc::clone(&app));
    let router = ServerBuilder::new(config)
        .add_api_routes(adk_surface::web::router(engine))
        .add_root_routes(adk_surface::web::home().merge(page))
        .build();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("Claims Desk on http://127.0.0.1:{port}/claims");
    println!("the adjuster's desk and each claim's thread on http://127.0.0.1:{port}/ui/");
    let stopping = Arc::new(tokio::sync::Notify::new());
    let server = tokio::spawn(
        axum::serve(listener, router)
            .with_graceful_shutdown({
                let stopping = Arc::clone(&stopping);
                async move { stopping.notified().await }
            })
            .into_future(),
    );
    let worker = {
        let app = Arc::clone(&app);
        tokio::spawn(async move {
            if let Err(err) = app.work().await {
                eprintln!("[claims] stopped: {err}");
            }
            println!("[claims] the day's claims are worked; the desk stays open until Ctrl-C");
        })
    };
    tokio::signal::ctrl_c().await?;
    worker.abort();
    stopping.notify_one();
    let _ = tokio::time::timeout(Duration::from_secs(3), server).await;
    desk.kernel.stop().await?;
    app.tape.save()?;
    Ok(())
}

async fn photo(State(app): State<Arc<App>>, Path(name): Path<String>) -> Response {
    if !app
        .claims
        .iter()
        .flat_map(|c| &c.photos)
        .any(|p| p.name == name)
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    match std::fs::read(data::photo_path(&crate::root(), &name)) {
        Ok(bytes) => ([(header::CONTENT_TYPE, "image/jpeg")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn state(State(app): State<Arc<App>>) -> impl IntoResponse {
    let started = app
        .started
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let (mut estimates, mut referred) = (0, 0);
    let mut rows = Vec::new();
    for claim in &app.claims {
        let (got, args) = if started.contains(&claim.id) {
            crate::outcome(&app.desk.calls(&claim.id).await)
        } else {
            (None, None)
        };
        let args = args.unwrap_or_default();
        let status = match got.as_deref() {
            Some("estimate") => {
                estimates += 1;
                format!(
                    "Estimate for the adjuster · {}",
                    args["severity"].as_str().unwrap_or_default()
                )
            }
            Some(reason) => {
                referred += 1;
                format!("Referred · {}", reason.replace('_', " "))
            }
            None if started.contains(&claim.id) => "Assessing".to_string(),
            None => "New".to_string(),
        };
        rows.push(json!({
            "id": claim.id,
            "vehicle": claim.vehicle,
            "date": claim.date,
            "story": claim.story,
            "photos": claim.photos.iter().map(|p| &p.name).collect::<Vec<_>>(),
            "status": status,
            "title": args["title"],
            "notes": args["notes"].as_str().or(args["detail"].as_str()),
            "lines": args["lines"],
        }));
    }
    let spend: f64 = app.tape.spend().values().map(|s| s.usd).sum();
    axum::Json(json!({
        "claims": rows,
        "estimates": estimates,
        "referred": referred,
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
<title>Claims Desk</title>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap">
<style>
:root { --bg:#fafaf9; --card:#fff; --raised:#f4f4f1; --border:#e7e6e2; --text:#18181b; --text2:#4b4b53; --label:#6b6b74; --btn:#c2410c; --btnText:#fff;
  --alert:#b42318; --alertBg:#fdecea; --info:#1d4ed8; --infoBg:#e8eefc; }
@media (prefers-color-scheme: dark) { :root { --bg:#0e0f11; --card:#16171a; --raised:#1e1f23; --border:#26272c; --text:#ededef; --text2:#b5b5bd; --label:#8e8e97;
  --btn:#ff6a1f; --btnText:#0e0f11; --alert:#ff8a80; --alertBg:#3a1d1b; --info:#8fb2ff; --infoBg:#1a2440; } }
* { box-sizing: border-box; }
body { margin:0; font-family: Inter, system-ui, sans-serif; background: var(--bg); color: var(--text); }
header { display:flex; flex-wrap:wrap; align-items:center; gap:12px 20px; padding:14px 28px; border-bottom:1px solid var(--border); background:var(--card); }
.logo { display:flex; align-items:center; gap:10px; font-weight:700; } .logo span { width:28px; height:28px; border-radius:7px; background:#f74c00; display:inline-block; }
.kpis { display:flex; gap:22px; font-size:13px; color:var(--text2); } .kpis b { color:var(--text); font-size:15px; }
header a.desk { margin-left:auto; font-weight:600; font-size:14px; padding:10px 16px; border-radius:8px; background:var(--btn); color:var(--btnText); text-decoration:none; }
main { max-width:1400px; margin:0 auto; padding:24px 28px; display:flex; flex-wrap:wrap; gap:24px; align-items:flex-start; }
.list { flex: 1 1 360px; min-width:0; background:var(--card); border:1px solid var(--border); border-radius:12px; overflow:hidden; }
.list h2, .view h2 { margin:0; padding:14px 16px 8px; font-size:15px; }
ol { list-style:none; margin:0; padding:0; max-height:74vh; overflow:auto; }
li { border-top:1px solid var(--border); padding:10px 16px; cursor:pointer; font-size:13px; display:flex; flex-direction:column; gap:3px; }
li:hover, li[aria-selected=true] { background:var(--raised); }
.tag { align-self:flex-start; font-size:11px; font-weight:700; padding:3px 8px; border-radius:999px; background:var(--border); color:var(--text2); }
.Estimate { background:var(--infoBg); color:var(--info); } .Referred { background:var(--alertBg); color:var(--alert); }
.view { flex: 2 1 560px; min-width:0; background:var(--card); border:1px solid var(--border); border-radius:12px; padding-bottom:16px; }
.photos { display:grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap:12px; padding:0 16px; }
.photos img { width:100%; border-radius:8px; display:block; }
.view p { margin:10px 16px; font-size:13px; line-height:1.5; color:var(--text2); }
table { margin:8px 16px; border-collapse:collapse; font-size:13px; width:calc(100% - 32px); }
td, th { border-top:1px solid var(--border); padding:6px 4px; text-align:left; } th { color:var(--label); font-weight:600; }
footer { padding:12px 28px; font-size:11px; color:var(--label); border-top:1px solid var(--border); }
@media (max-width: 720px) { main { padding:16px; } header { padding:14px 16px; } }
</style>
</head>
<body>
<header><div class="logo"><span aria-hidden="true"></span>Claims Desk</div>
<div class="kpis"><span><b id="estimates">0</b> estimates</span><span><b id="referred">0</b> referred</span><span><b id="spend">$0</b> <span id="spend-label">model spend</span></span></div>
<a class="desk" href="/ui/">Adjuster's desk</a></header>
<main>
<section class="list" aria-label="Claims"><h2>Today's claims</h2><ol id="list"></ol></section>
<section class="view" aria-label="Claim"><h2 id="title">Choose a claim</h2><div class="photos" id="photos"></div><p id="story"></p><p id="notes"></p><table id="lines"></table></section>
</main>
<footer>Photos: Wikimedia Commons, each under its own free licence; authors and licences in DATA.md. Prices are illustrative.</footer>
<script>
let rows = [], chosen = null;
function esc(s) { return String(s ?? '').replace(/[&<>"]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c])); }
function show(id) {
  chosen = id; const r = rows.find(x => x.id === id); if (!r) return;
  document.getElementById('title').textContent = r.id + ' · ' + r.vehicle + ' · ' + r.status;
  document.getElementById('photos').innerHTML = r.photos.map(p => `<img src="/claims/photo/${esc(p)}" alt="${esc(p)}">`).join('');
  document.getElementById('story').textContent = 'What happened (' + r.date + '): ' + r.story;
  document.getElementById('notes').textContent = r.notes || '';
  document.getElementById('lines').innerHTML = Array.isArray(r.lines) && r.lines.length ? '<tr><th>Part</th><th>Action</th><th>Hours</th></tr>' + r.lines.map(l => `<tr><td>${esc(l.part)}</td><td>${esc(l.action)}</td><td>${esc(l.hours)}</td></tr>`).join('') : '';
  document.querySelectorAll('#list li').forEach(li => li.setAttribute('aria-selected', li.dataset.id === id));
}
async function tick() {
  try {
    const s = await (await fetch('/claims/state')).json(); rows = s.claims;
    document.getElementById('estimates').textContent = s.estimates; document.getElementById('referred').textContent = s.referred;
    document.getElementById('spend').textContent = '$' + s.spendUsd.toFixed(2); document.getElementById('spend-label').textContent = s.spendLabel;
    document.getElementById('list').innerHTML = rows.map(r => `<li data-id="${esc(r.id)}" tabindex="0"><b>${esc(r.id)} · ${esc(r.vehicle)}</b><span class="tag ${esc(r.status.split(' ')[0])}">${esc(r.status)}</span></li>`).join('');
    document.querySelectorAll('#list li').forEach(li => { li.onclick = () => show(li.dataset.id); li.onkeydown = e => { if (e.key === 'Enter') show(li.dataset.id); }; });
    if (chosen) show(chosen);
  } catch (e) {}
}
tick(); setInterval(tick, 1000);
</script>
</body>
</html>
"##;
