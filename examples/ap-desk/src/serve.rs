//! The web home while the inbox is worked: each invoice's turn as a thread, the exceptions on
//! the clerk's desk with the page as evidence, and the AP Desk page with the inbox, each
//! page and what was read from it.

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
use serde_json::{Value, json};

use crate::desk::{APP, USER};
use crate::{App, Common, data};

struct Clerk;

#[async_trait]
impl RequestContextExtractor for Clerk {
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

/// The heading an exception card shows on the clerk's desk.
struct ExceptionCards;

#[async_trait]
impl CardHints for ExceptionCards {
    async fn hint(&self, request: &adk_core::ApprovalRequest) -> Option<CardHint> {
        (request.tool_name == "raise_exception").then(|| {
            let doc = request.args_redacted["doc"].as_str().unwrap_or_default();
            let title = request.args_redacted["title"]
                .as_str()
                .unwrap_or("Invoice to check");
            CardHint::new().with_heading(format!("{doc}: {title}"))
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
        .agent_name(APP, "AP Desk")
        .card_hints(Arc::new(ExceptionCards))
        .build();
    let config = ServerConfig::new(
        Arc::new(SingleAgentLoader::new(desk.agent.clone())),
        desk.sessions.clone(),
    )
    .with_request_context(Arc::new(Clerk));
    let page = axum::Router::new()
        .route("/ap", get(|| async { Html(PAGE) }))
        .route("/ap/state", get(state))
        .route("/ap/page/{doc}", get(page_image))
        .with_state(Arc::clone(&app));
    let router = ServerBuilder::new(config)
        .add_api_routes(adk_surface::web::router(engine))
        .add_root_routes(adk_surface::web::home().merge(page))
        .build();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("AP Desk on http://127.0.0.1:{port}/ap");
    println!("the clerk's desk and each invoice's thread on http://127.0.0.1:{port}/ui/");
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
        let limit = common.docs;
        tokio::spawn(async move {
            if let Err(err) = app.work(limit).await {
                eprintln!("[inbox] stopped: {err}");
            }
            println!("[inbox] worked; the desk stays open until Ctrl-C");
        })
    };
    tokio::signal::ctrl_c().await?;
    println!();
    println!("stopping…");
    worker.abort();
    stopping.notify_one();
    let _ = tokio::time::timeout(Duration::from_secs(3), server).await;
    desk.kernel.stop().await?;
    app.tape.save()?;
    Ok(())
}

async fn page_image(State(app): State<Arc<App>>, Path(doc): Path<String>) -> Response {
    let doc = doc.trim_end_matches(".jpg");
    let known = app.books.expected.iter().any(|e| e.doc == doc);
    if !known {
        return StatusCode::NOT_FOUND.into_response();
    }
    match std::fs::read(data::inbox_dir(&crate::root()).join(format!("{doc}.jpg"))) {
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
    let out = app.desk.ledger_dir();
    let (mut posted, mut held) = (0, 0);
    let mut rows = Vec::new();
    for expected in &app.books.expected {
        let calls = if started.contains(&expected.doc) {
            app.desk.calls(&expected.doc).await
        } else {
            Vec::new()
        };
        let outcome = crate::eval::Outcome::from_calls(&expected.doc, &calls);
        let accepted = out
            .join("exceptions")
            .join(format!("{}.json", expected.doc))
            .exists();
        let status = match outcome.decision.as_deref() {
            Some("post") => {
                posted += 1;
                "Posted".to_string()
            }
            Some(kind) => {
                held += 1;
                if accepted {
                    format!("Exception accepted · {}", kind.replace('_', " "))
                } else {
                    format!("On the clerk's desk · {}", kind.replace('_', " "))
                }
            }
            None if started.contains(&expected.doc) => "Reading".to_string(),
            None => "In the inbox".to_string(),
        };
        let fields = outcome.fields.unwrap_or(Value::Null);
        rows.push(json!({
            "doc": expected.doc,
            "status": status,
            "supplier": fields["supplier"],
            "number": fields["number"],
            "total": fields["total"],
            "currency": fields["currency"],
            "po": fields["po"],
            "date": fields["date"],
            "crops": outcome.crops,
        }));
    }
    let spend: f64 = app.tape.spend().values().map(|s| s.usd).sum();
    axum::Json(json!({
        "documents": rows,
        "posted": posted,
        "held": held,
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
<title>AP Desk</title>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap">
<style>
:root { --bg:#fafaf9; --card:#fff; --raised:#f4f4f1; --border:#e7e6e2; --text:#18181b; --text2:#4b4b53; --label:#6b6b74;
  --btn:#c2410c; --btnText:#fff; --alert:#b42318; --alertBg:#fdecea; --ok:#1e6b45; --okBg:#e7f4ec; --info:#1d4ed8; --infoBg:#e8eefc; }
@media (prefers-color-scheme: dark) { :root { --bg:#0e0f11; --card:#16171a; --raised:#1e1f23; --border:#26272c; --text:#ededef; --text2:#b5b5bd;
  --label:#8e8e97; --btn:#ff6a1f; --btnText:#0e0f11; --alert:#ff8a80; --alertBg:#3a1d1b; --ok:#7ad3a0; --okBg:#17301f; --info:#8fb2ff; --infoBg:#1a2440; } }
* { box-sizing: border-box; }
body { margin:0; font-family: Inter, system-ui, sans-serif; background: var(--bg); color: var(--text); }
header { display:flex; flex-wrap:wrap; align-items:center; gap:12px 20px; padding:14px 28px; border-bottom:1px solid var(--border); background:var(--card); }
.logo { display:flex; align-items:center; gap:10px; font-weight:700; }
.logo span { width:28px; height:28px; border-radius:7px; background:#f74c00; display:inline-block; }
.kpis { display:flex; gap:22px; font-size:13px; color:var(--text2); }
.kpis b { color:var(--text); font-size:15px; }
header a.desk { margin-left:auto; font-weight:600; font-size:14px; padding:10px 16px; border-radius:8px; background:var(--btn); color:var(--btnText); text-decoration:none; }
main { max-width:1400px; margin:0 auto; padding:24px 28px; display:flex; flex-wrap:wrap; gap:24px; align-items:flex-start; }
.inbox { flex: 1 1 380px; min-width:0; background:var(--card); border:1px solid var(--border); border-radius:12px; overflow:hidden; }
.inbox h2, .viewer h2 { margin:0; padding:14px 16px 8px; font-size:15px; }
.inbox ol { list-style:none; margin:0; padding:0; max-height:72vh; overflow:auto; }
.inbox li { border-top:1px solid var(--border); padding:10px 16px; display:grid; grid-template-columns: 44px minmax(0,1fr) auto; gap:4px 10px; cursor:pointer; font-size:13px; }
.inbox li:hover, .inbox li[aria-selected=true] { background:var(--raised); }
.inbox li b { font-size:13px; }
.inbox .sub { grid-column: 2 / 4; color:var(--text2); font-size:12px; }
.tag { font-size:11px; font-weight:700; padding:3px 8px; border-radius:999px; white-space:nowrap; align-self:start; }
.Posted { background:var(--okBg); color:var(--ok); } .On { background:var(--alertBg); color:var(--alert); }
.Exception { background:var(--infoBg); color:var(--info); } .Reading, .In { background:var(--border); color:var(--text2); }
.viewer { flex: 2 1 560px; min-width:0; background:var(--card); border:1px solid var(--border); border-radius:12px; display:flex; flex-wrap:wrap; gap:16px; padding:0 0 16px; }
.viewer h2 { width:100%; }
.viewer img { flex: 1 1 320px; min-width:0; max-width:100%; margin-left:16px; border:1px solid var(--border); border-radius:6px; background:#fff; }
.viewer dl { flex: 1 1 220px; margin:0 16px; display:grid; grid-template-columns:auto 1fr; gap:8px 12px; font-size:13px; align-content:start; }
.viewer dt { color:var(--label); }
.viewer dd { margin:0; font-weight:600; overflow-wrap:anywhere; }
footer { padding:12px 28px; font-size:11px; color:var(--label); border-top:1px solid var(--border); }
@media (max-width: 720px) { main { padding:16px; } header { padding:14px 16px; } .viewer img { margin:0 16px; } }
</style>
</head>
<body>
<header><div class="logo"><span aria-hidden="true"></span>AP Desk</div>
<div class="kpis"><span><b id="posted">0</b> posted</span><span><b id="held">0</b> for the clerk</span><span><b id="spend">$0</b> <span id="spend-label">model spend</span></span></div>
<a class="desk" href="/ui/">Clerk's desk</a></header>
<main>
<section class="inbox" aria-label="Inbox"><h2>Inbox</h2><ol id="list"></ol></section>
<section class="viewer" aria-label="Invoice"><h2 id="title">Choose an invoice</h2><img id="page" alt="" hidden><dl id="fields"></dl></section>
</main>
<footer>Invoices: FATURA (Limam et al.), Zenodo record 8261508, CC BY 4.0; synthetic invoices with invented companies. The ERP is generated.</footer>
<script>
let chosen = null, rows = [];
function esc(s) { return String(s ?? '').replace(/[&<>"]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c])); }
function show(doc) {
  chosen = doc; const r = rows.find(x => x.doc === doc); if (!r) return;
  document.getElementById('title').textContent = r.doc + ' · ' + r.status;
  const img = document.getElementById('page'); img.hidden = false; img.src = '/ap/page/' + r.doc + '.jpg'; img.alt = 'Invoice ' + r.doc;
  const f = [['Supplier', r.supplier], ['Invoice number', r.number], ['Date', r.date], ['PO', r.po || '—'], ['Total', r.total != null ? r.total + ' ' + (r.currency || '') : ''], ['Crops sent to Claude', r.crops]];
  document.getElementById('fields').innerHTML = f.map(([k, v]) => `<dt>${esc(k)}</dt><dd>${esc(v)}</dd>`).join('');
  document.querySelectorAll('#list li').forEach(li => li.setAttribute('aria-selected', li.dataset.doc === doc));
}
async function tick() {
  try {
    const s = await (await fetch('/ap/state')).json(); rows = s.documents;
    document.getElementById('posted').textContent = s.posted;
    document.getElementById('held').textContent = s.held;
    document.getElementById('spend').textContent = '$' + s.spendUsd.toFixed(2);
    document.getElementById('spend-label').textContent = s.spendLabel;
    document.getElementById('list').innerHTML = rows.map(r => `<li data-doc="${esc(r.doc)}" tabindex="0"><b>${esc(r.doc)}</b><span>${esc(r.supplier || '')}</span><span class="tag ${esc(r.status.split(' ')[0].replace(',', ''))}">${esc(r.status)}</span><span class="sub">${r.total != null ? esc(r.total + ' ' + (r.currency || '')) : ''}${r.number ? ' · ' + esc(r.number) : ''}</span></li>`).join('');
    document.querySelectorAll('#list li').forEach(li => { li.onclick = () => show(li.dataset.doc); li.onkeydown = e => { if (e.key === 'Enter') show(li.dataset.doc); }; });
    if (chosen) show(chosen);
  } catch (e) {}
}
tick(); setInterval(tick, 1000);
</script>
</body>
</html>
"##;
