//! The Claims Desk page while the day's claims are worked: each claim's photos and what the
//! desk found, the adjuster's decision on its estimate or referral, and the claim's
//! conversation with the desk. The web home stays at `/ui/` as the developer view.

use std::collections::BTreeMap;
use std::future::IntoFuture;
use std::sync::Arc;
use std::time::Duration;

use adk_artifact::ArtifactService;
use adk_core::{ApprovalState, InboxQuery, RequestContext, RunFilter, RunState, SingleAgentLoader};
use adk_server::auth_bridge::{RequestContextError, RequestContextExtractor};
use adk_server::{ServerBuilder, ServerConfig};
use adk_surface::{CardHint, CardHints, LocalSource, Place, SurfaceEngine};
use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::data::PRICES;
use crate::desk::{APP, USER};
use crate::tape::{Miss, Mode};
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

/// Where the day is: `serve` waits for the adjuster to press Start the day.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum Day {
    Waiting,
    Running,
    Finished,
}

struct Page {
    app: Arc<App>,
    day: watch::Sender<Day>,
    daily_usd: f64,
}

pub async fn serve(common: Common, port: u16) -> anyhow::Result<()> {
    let app = Arc::new(App::build(&common, Miss::Say).await?);
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
    let page = Arc::new(Page {
        app: Arc::clone(&app),
        day: watch::channel(Day::Waiting).0,
        daily_usd: common.daily_usd,
    });
    let routes = axum::Router::new()
        .route("/claims", get(|| async { Html(PAGE) }))
        .route("/claims/state", get(state))
        .route("/claims/start", post(start))
        .route("/claims/photo/{name}", get(photo))
        .with_state(Arc::clone(&page));
    let router = ServerBuilder::new(config)
        .add_api_routes(adk_surface::web::router(engine))
        .add_root_routes(adk_surface::web::home().merge(routes))
        .build();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!(
        "Claims Desk on http://127.0.0.1:{port}/claims: the day starts when you press Start the day"
    );
    println!("the developer view, ADK-Rust's web home, on http://127.0.0.1:{port}/ui/");
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
        let page = Arc::clone(&page);
        tokio::spawn(async move {
            let mut day = page.day.subscribe();
            if day.wait_for(|day| *day == Day::Running).await.is_err() {
                return;
            }
            if let Err(err) = page.app.work().await {
                eprintln!("[claims] stopped: {err}");
            }
            page.day.send_replace(Day::Finished);
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

/// Starts the day once; pressing it again, or after the day, changes nothing.
async fn start(State(page): State<Arc<Page>>) -> impl IntoResponse {
    page.day.send_if_modified(|day| {
        let waiting = *day == Day::Waiting;
        if waiting {
            *day = Day::Running;
        }
        waiting
    });
    axum::Json(json!({ "day": *page.day.borrow() }))
}

async fn photo(State(page): State<Arc<Page>>, Path(name): Path<String>) -> Response {
    if !page
        .app
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

/// The day and every claim as the page draws them: what the desk found, the estimate priced
/// line by line, a referral with the photo it is about, and where the claim's turn is.
async fn state(State(page): State<Arc<Page>>) -> impl IntoResponse {
    let app = &page.app;
    let started = app
        .started
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let worked = app.worked.lock().unwrap_or_else(|e| e.into_inner()).clone();
    // Why the adjuster declined, by claim: the decision card says who and when, not why.
    let declined: BTreeMap<String, String> = app
        .desk
        .approvals
        .inbox(
            InboxQuery::for_group("owner")
                .with_app_name(APP)
                .with_states([ApprovalState::Rejected])
                .with_limit(200),
        )
        .await
        .map(|page| {
            page.items
                .into_iter()
                .filter_map(|r| Some((r.scope.session_id, r.decisions.last()?.reason.clone()?)))
                .collect()
        })
        .unwrap_or_default();
    let (mut estimates, mut referred) = (0, 0);
    let mut rows = Vec::new();
    for claim in &app.claims {
        let (got, args) = if started.contains(&claim.id) {
            crate::outcome(&app.desk.calls(&claim.id).await)
        } else {
            (None, None)
        };
        let args = args.unwrap_or_default();
        let (mut estimate, mut referral) = (Value::Null, Value::Null);
        match got.as_deref() {
            Some("estimate") => {
                estimates += 1;
                let lines: Vec<Value> = args["lines"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .map(|line| {
                        let hours = line["hours"].as_f64().unwrap_or(0.0);
                        // Priced as `draft_estimate` prices it once approved; the illustrative
                        // list names no currency, so neither does the page.
                        let part = if line["action"] == "replace" {
                            crate::desk::price(line["part"].as_str().unwrap_or_default())
                        } else {
                            0.0
                        };
                        let labour = hours * PRICES.labour_per_hour;
                        json!({ "part": line["part"], "action": line["action"], "hours": hours,
                                "parts": part, "labour": labour, "amount": part + labour })
                    })
                    .collect();
                let sum = |field: &str| lines.iter().filter_map(|l| l[field].as_f64()).sum::<f64>();
                estimate = json!({
                    "severity": args["severity"],
                    "sides": args["sides"],
                    "hours": sum("hours"),
                    "parts": sum("parts"),
                    "labour": sum("labour"),
                    "total": sum("amount"),
                    "labourRate": PRICES.labour_per_hour,
                    "lines": lines,
                });
            }
            Some(reason) => {
                referred += 1;
                let photo = args["photo"].as_str().unwrap_or_default();
                let earlier = if reason == "reused_photo" {
                    app.desk
                        .memory
                        .found(&format!("{}/{photo}", claim.id))
                        .into_iter()
                        .next()
                        .map(|(key, similarity)| {
                            let (claim, photo) = key.split_once('/').unwrap_or((key.as_str(), ""));
                            json!({ "claim": claim, "photo": photo, "similarity": similarity })
                        })
                } else {
                    None
                };
                referral = json!({
                    "reason": reason,
                    "photo": photo,
                    "detail": args["detail"],
                    "earlier": earlier,
                    "other": claim.photos.iter().map(|p| &p.name).find(|n| n.as_str() != photo),
                });
            }
            None => {}
        }
        let runs = app
            .desk
            .runs
            .list(RunFilter::default().app_name(APP).session_id(&claim.id))
            .await
            .unwrap_or_default();
        let turn = match runs.last() {
            None => json!({ "state": "idle", "runs": 0 }),
            Some(run) => {
                let state = match run.state {
                    RunState::Queued | RunState::Running if got.is_none() && runs.len() == 1 => {
                        "assessing"
                    }
                    RunState::Queued | RunState::Running => "answering",
                    RunState::Waiting => "waiting",
                    RunState::Failed => "failed",
                    // Completed or cancelled; `RunState` is non-exhaustive.
                    _ => "idle",
                };
                json!({ "state": state, "runs": runs.len(),
                        "error": if run.state == RunState::Failed { run.error.clone() } else { None } })
            }
        };
        let stage = if worked.contains(&claim.id) {
            "worked"
        } else if started.contains(&claim.id) {
            "assessing"
        } else {
            "new"
        };
        rows.push(json!({
            "id": claim.id,
            "policy": claim.policy,
            "vehicle": claim.vehicle,
            "date": claim.date,
            "story": claim.story,
            "photos": claim.photos.iter().map(|p| &p.name).collect::<Vec<_>>(),
            "task": crate::desk::task(claim),
            "stage": stage,
            "outcome": got,
            "title": args["title"],
            "notes": args["notes"].as_str().or(args["detail"].as_str()),
            "estimate": estimate,
            "referral": referral,
            "declineReason": declined.get(&claim.id),
            "turn": turn,
        }));
    }
    let spend: f64 = app.tape.spend().values().map(|s| s.usd).sum();
    axum::Json(json!({
        "mode": if app.tape.mode == Mode::Live { "live" } else { "replay" },
        "day": *page.day.borrow(),
        "worked": worked.len(),
        "total": app.claims.len(),
        "estimates": estimates,
        "referred": referred,
        "spendUsd": (spend * 10_000.0).round() / 10_000.0,
        "dailyUsd": page.daily_usd,
        "claims": rows,
    }))
}

const PAGE: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="icon" href="data:,">
<title>Claims Desk</title>
<style>
:root { --bg:#fafaf9; --card:#fff; --raised:#f4f4f1; --border:#e7e6e2; --text:#18181b; --text2:#4b4b53; --label:#6b6b74;
  --brand:#f74c00; --btn:#c2410c; --btnText:#fff; --btn2:#fff; --btn2Text:#18181b;
  --alert:#b42318; --alertBg:#fdecea; --info:#1d4ed8; --infoBg:#e8eefc; --ok:#15803d; --okBg:#e7f6ec; --warn:#a15c07; --warnBg:#fdf3e1; color-scheme:light; }
@media (prefers-color-scheme: dark) { :root { --bg:#0e0f11; --card:#16171a; --raised:#1e1f23; --border:#2a2b31; --text:#ededef; --text2:#b5b5bd; --label:#8e8e97;
  --btn:#ff6a1f; --btnText:#0e0f11; --btn2:#1e1f23; --btn2Text:#ededef;
  --alert:#ff8a80; --alertBg:#3a1d1b; --info:#8fb2ff; --infoBg:#1a2440; --ok:#6ee7a0; --okBg:#123222; --warn:#f5c26b; --warnBg:#3a2a10; color-scheme:dark; } }
* { box-sizing:border-box; }
[hidden] { display:none !important; }
body { margin:0; font:14px/1.5 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif; background:var(--bg); color:var(--text); }
button { font:inherit; font-weight:600; cursor:pointer; border-radius:8px; padding:9px 16px; border:1px solid var(--border); background:var(--btn2); color:var(--btn2Text); }
button.primary { background:var(--btn); color:var(--btnText); border-color:transparent; }
button:disabled { cursor:default; opacity:.55; }
button:focus-visible, input:focus-visible, textarea:focus-visible, a:focus-visible, summary:focus-visible { outline:2px solid var(--info); outline-offset:2px; }
code { font:12px ui-monospace, SFMono-Regular, Menlo, monospace; background:var(--raised); padding:1px 5px; border-radius:4px; }
.sr { position:absolute; width:1px; height:1px; overflow:hidden; clip:rect(0 0 0 0); white-space:nowrap; }
header.top { display:grid; grid-template-columns:auto 1fr auto; gap:10px 24px; align-items:center; padding:14px 28px; border-bottom:1px solid var(--border); background:var(--card); }
.brand { display:flex; align-items:center; gap:10px; } .brand h1 { margin:0; font-size:17px; } .logo { width:28px; height:28px; border-radius:7px; background:var(--brand); }
.mode { display:flex; align-items:center; gap:10px; min-width:0; color:var(--text2); font-size:13px; }
.badge { font-size:11px; font-weight:700; letter-spacing:.04em; text-transform:uppercase; padding:3px 9px; border-radius:999px; background:var(--infoBg); color:var(--info); white-space:nowrap; }
.badge.live { background:var(--okBg); color:var(--ok); }
.day { display:flex; flex-direction:column; align-items:flex-end; gap:2px; } .day small { color:var(--label); font-size:12px; }
.counts { grid-column:1 / -1; display:flex; flex-wrap:wrap; gap:6px 22px; margin:0; font-size:13px; color:var(--text2); }
.counts b { color:var(--text); font-size:15px; font-variant-numeric:tabular-nums; }
main { max-width:1400px; margin:0 auto; padding:24px 28px; display:flex; flex-wrap:wrap; gap:24px; align-items:flex-start; }
.offline { flex-basis:100%; margin:0; padding:10px 14px; border-radius:10px; background:var(--alertBg); color:var(--alert); font-size:13px; }
.list { flex:1 1 320px; min-width:0; background:var(--card); border:1px solid var(--border); border-radius:12px; overflow:hidden; }
.list h2 { margin:0; padding:14px 16px 6px; font-size:15px; }
.list .note { margin:0; padding:0 16px 12px; font-size:13px; color:var(--text2); }
ol { list-style:none; margin:0; padding:0; max-height:76vh; overflow:auto; }
ol li { border-top:1px solid var(--border); }
ol button { all:unset; box-sizing:border-box; display:flex; flex-direction:column; gap:4px; width:100%; padding:10px 16px; cursor:pointer; }
ol button:hover, ol button[aria-current=true] { background:var(--raised); }
ol button:focus-visible { outline:2px solid var(--info); outline-offset:-2px; }
.row1 { font-size:13px; } .row1 span { color:var(--text2); }
.row2 { display:flex; flex-wrap:wrap; gap:6px; }
.tag { font-size:11px; font-weight:700; padding:3px 8px; border-radius:999px; background:var(--raised); color:var(--text2); white-space:nowrap; }
.tag.estimate { background:var(--infoBg); color:var(--info); } .tag.referred { background:var(--alertBg); color:var(--alert); }
.tag.wait { background:var(--warnBg); color:var(--warn); } .tag.yes { background:var(--okBg); color:var(--ok); }
.view { flex:2 1 560px; min-width:0; background:var(--card); border:1px solid var(--border); border-radius:12px; padding:4px 20px 20px; }
.empty { color:var(--text2); padding:20px 0 4px; margin:0; }
.claim-head { display:flex; flex-wrap:wrap; align-items:center; gap:8px 12px; padding:14px 0 0; } .claim-head h2 { margin:0; font-size:17px; }
.view h3 { margin:24px 0 8px; font-size:12px; text-transform:uppercase; letter-spacing:.05em; color:var(--label); }
.photos { display:grid; grid-template-columns:repeat(auto-fill, minmax(200px, 340px)); gap:12px; }
.pair { display:grid; grid-template-columns:repeat(auto-fit, minmax(200px, 1fr)); gap:12px; margin-top:10px; }
figure { margin:0; } figure img { width:100%; aspect-ratio:4 / 3; object-fit:cover; border-radius:8px; display:block; background:var(--raised); }
figcaption { font-size:12px; color:var(--label); margin-top:4px; }
dl.facts { display:grid; grid-template-columns:max-content 1fr; gap:4px 16px; margin:12px 0 0; font-size:13px; } dl.facts dt { color:var(--label); } dl.facts dd { margin:0; }
.finding { margin-top:12px; padding:12px 14px; border-radius:10px; background:var(--raised); font-size:13px; } .finding p { margin:4px 0; } .finding .title { font-weight:600; }
.muted { color:var(--text2); }
.card { border:1px solid var(--border); border-radius:10px; padding:14px; font-size:13px; } .card.waiting { border-color:var(--warn); }
.card p { margin:8px 0 0; }
.card-head { display:flex; flex-wrap:wrap; justify-content:space-between; gap:6px 12px; font-size:14px; } .card-head .since { font-size:12px; color:var(--label); font-weight:400; }
table { border-collapse:collapse; width:100%; margin:10px 0 0; font-variant-numeric:tabular-nums; }
th, td { border-top:1px solid var(--border); padding:6px 4px; text-align:left; } th { color:var(--label); font-weight:600; } .n { text-align:right; }
tfoot td { font-weight:700; }
.fine { font-size:12px; color:var(--label); }
.actions { display:flex; flex-wrap:wrap; gap:10px; margin-top:14px; }
.decline { margin-top:14px; } .decline label { font-size:13px; color:var(--text2); display:flex; flex-direction:column; gap:4px; }
textarea, input[type=text] { font:inherit; color:var(--text); background:var(--bg); border:1px solid var(--border); border-radius:8px; padding:9px 10px; width:100%; }
textarea { min-height:64px; resize:vertical; }
.decided { margin:0; font-weight:600; } .decided.no { color:var(--text2); }
details { margin-top:8px; } summary { cursor:pointer; color:var(--text2); }
.status-note { font-size:13px; margin:8px 0 0; color:var(--text2); } .status-note.err { color:var(--alert); } .status-note:empty { display:none; }
.turn { margin:12px 0; display:flex; flex-direction:column; gap:6px; } .sys, .steps { font-size:12px; color:var(--label); margin:0; }
.msg { max-width:88%; padding:9px 12px; border-radius:10px; font-size:13px; white-space:pre-wrap; overflow-wrap:anywhere; }
.msg .who { display:block; font-size:11px; font-weight:700; color:var(--label); margin-bottom:2px; }
.msg.agent { background:var(--raised); align-self:flex-start; } .msg.you { background:var(--infoBg); align-self:flex-end; }
form.ask { display:flex; gap:8px; margin-top:14px; } form.ask input { flex:1; min-width:0; }
footer { max-width:1400px; margin:0 auto; padding:12px 28px 28px; font-size:12px; color:var(--label); display:flex; flex-wrap:wrap; gap:6px 18px; }
footer a { color:var(--text2); font-weight:600; }
@media (max-width:720px) {
  header.top { grid-template-columns:1fr; padding:14px 16px; } .day { align-items:flex-start; }
  main { padding:16px; gap:16px; } .view { padding:4px 16px 16px; } footer { padding:12px 16px 24px; } ol { max-height:40vh; }
}
</style>
</head>
<body>
<header class="top">
  <div class="brand"><span class="logo" aria-hidden="true"></span><h1>Claims Desk</h1></div>
  <div class="mode"><span class="badge" id="badge"></span><span id="mode-line"></span></div>
  <div class="day"><button type="button" class="primary" id="start" disabled>Start the day</button><small id="start-hint"></small></div>
  <p class="counts" id="counts"></p>
</header>
<main>
  <p class="offline" id="offline" role="alert" hidden>The desk is not answering. Is the app still running?</p>
  <section class="list" aria-labelledby="list-h"><h2 id="list-h">Today's claims</h2><p class="note" id="list-note"></p><ol id="list"></ol></section>
  <section class="view" aria-label="The chosen claim">
    <p class="empty" id="empty">Choose a claim to see its photos, what the desk found and your decision.</p>
    <article id="claim" hidden>
      <div class="claim-head"><h2 id="c-title"></h2><span class="tag" id="c-status"></span></div>
      <h3>What the desk found</h3><div id="found"></div>
      <div id="decision-wrap" hidden><h3>Your decision</h3><div id="decision"></div></div>
      <h3 id="conv-h">Conversation</h3><div id="conversation"></div>
      <form class="ask" id="ask"><label class="sr" for="ask-input">Your question about this claim</label><input type="text" id="ask-input" autocomplete="off" maxlength="2000"><button type="submit" class="primary" id="ask-send">Send</button></form>
      <p class="status-note" id="ask-status" aria-live="polite"></p>
      <p class="fine" id="ask-note"></p>
    </article>
  </section>
</main>
<footer><a href="/ui/">Developer view</a><span>Photos: Wikimedia Commons, each under its own free licence; authors and licences in DATA.md. Prices are illustrative.</span></footer>
<script>
const $ = id => document.getElementById(id);
let S = null, cards = {}, chosen = decodeURIComponent(location.hash.slice(1)) || null, thread = null;
let declining = null, sending = false, ticking = null, offlineSince = 0;
const painted = {}, decisionNote = {}, askNote = {}, awaiting = {};
const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const enc = encodeURIComponent;
const amount = n => Number(n || 0).toLocaleString('en-US', {minimumFractionDigits: 2, maximumFractionDigits: 2});
const usd = n => '$' + amount(n);
const plural = (n, one, many) => n + ' ' + (n === 1 ? one : many);
const row = id => S && S.claims.find(c => c.id === id);
const photo = (name, caption, alt) => `<figure><img src="/claims/photo/${enc(name)}" alt="${esc(alt)}" loading="lazy"><figcaption>${esc(caption)}</figcaption></figure>`;
function paint(id, html) { if (painted[id] === html) return; painted[id] = html; $(id).innerHTML = html; }
async function getJson(url) { const r = await fetch(url, {cache: 'no-store'}); if (!r.ok) throw new Error(url + ' answered ' + r.status); return r.json(); }
async function command(command) {
  const r = await fetch('/api/surface/commands', {method: 'POST', headers: {'content-type': 'application/json'},
    body: JSON.stringify({surface: 'web', origin: {kind: 'pointer'}, command})});
  let body = {}; try { body = await r.json(); } catch (e) { body = {}; }
  return {status: r.status, ok: r.ok, body};
}
const problem = res => res.body.detail || res.body.title || ('the desk answered ' + res.status);
function dur(ms) { return ms < 60000 ? Math.max(1, Math.round(ms / 1000)) + ' s' : Math.round(ms / 60000) + ' min'; }

// The open decision of each claim, else its latest answered one.
function byClaim(list) {
  const m = {};
  for (const c of list) {
    if (c.call.tool !== 'draft_estimate' && c.call.tool !== 'refer_claim') continue;
    const had = m[c.sessionId];
    if (!had || (had.status !== 'waiting' && c.status === 'waiting')) m[c.sessionId] = c;
  }
  return m;
}
function statusOf(r) {
  if (r.outcome === 'estimate') return ['Estimate · ' + (r.estimate.severity || ''), 'estimate'];
  if (r.outcome === 'reused_photo') return ['Referred · reused photo', 'referred'];
  if (r.outcome === 'different_vehicle') return ['Referred · another vehicle', 'referred'];
  if (r.outcome) return ['Referred', 'referred'];
  if (r.stage === 'assessing') return ['Assessing…', ''];
  if (r.stage === 'worked') return ['No estimate or referral', ''];
  return [S.day === 'waiting' ? 'Not started' : 'In the queue', ''];
}
function pillOf(card) {
  if (!card) return null;
  if (card.status === 'waiting') return ['Your decision', 'wait'];
  const a = card.decided && card.decided.answer;
  return a === 'yes' ? ['Approved', 'yes'] : a === 'no' ? ['Declined', ''] : ['Closed', ''];
}

function renderHeader() {
  const live = S.mode === 'live';
  $('badge').textContent = live ? 'Live' : 'Replay';
  $('badge').className = 'badge ' + S.mode;
  $('mode-line').textContent = live ? 'Claude answers as you watch'
    : 'A recorded day played back: no key, no spend. Decisions work; new questions need a live run.';
  const waiting = Object.values(cards).filter(c => c.status === 'waiting').length;
  paint('counts', `<span><b>${S.worked} of ${S.total}</b> claims worked</span><span><b>${S.estimates}</b> ${S.estimates === 1 ? 'estimate' : 'estimates'}</span>`
    + `<span><b>${S.referred}</b> referred</span><span><b>${waiting}</b> waiting for your decision</span>`
    + `<span><b>${usd(S.spendUsd)}</b> ${live ? 'of ' + usd(S.dailyUsd) : 'when it was recorded'}</span>`);
  const start = $('start'), hint = $('start-hint');
  if (S.day === 'waiting') { start.disabled = false; start.textContent = 'Start the day'; hint.textContent = plural(S.total, 'claim waits', 'claims wait'); }
  else if (S.day === 'running') { start.disabled = true; start.textContent = 'Day started'; hint.textContent = 'the desk works the claims in the order they arrived'; }
  else { start.disabled = true; start.textContent = 'Day finished'; hint.textContent = 'restart the app to run it again'; }
}

function renderList() {
  const focused = document.activeElement && document.activeElement.dataset ? document.activeElement.dataset.id : null;
  paint('list', S.claims.map(r => {
    const [status, cls] = statusOf(r), pill = pillOf(cards[r.id]);
    return `<li><button type="button" data-id="${esc(r.id)}" aria-current="${r.id === chosen}"><span class="row1"><b>${esc(r.id)}</b> <span>· ${esc(r.vehicle)}</span></span>`
      + `<span class="row2"><span class="tag ${cls}">${esc(status)}</span>${pill ? `<span class="tag ${pill[1]}">${pill[0]}</span>` : ''}</span></button></li>`;
  }).join(''));
  if (focused && document.activeElement === document.body) { const b = document.querySelector(`#list button[data-id="${focused}"]`); if (b) b.focus(); }
  paint('list-note', S.day === 'waiting'
    ? `The day has not started. Press <b>Start the day</b> and the desk works the ${S.total} claims in the order they arrived; each estimate or referral then waits for your decision.`
    : S.day === 'running' ? 'The desk works the claims in the order they arrived. Choose one to follow it.'
    : 'Every claim is worked. A claim marked Your decision waits for you.');
}

function foundHtml(r) {
  const photos = r.photos.map((p, i) => photo(p, `Photo ${i + 1} · ${p}`, `Photo ${i + 1} of claim ${r.id}`)).join('');
  let finding;
  if (r.outcome) {
    const e = r.estimate;
    const meta = e ? `<p class="muted">Severity ${esc(e.severity)}${(e.sides || []).length ? ' · damaged ' + esc(e.sides.join(', ')) : ''}</p>` : '';
    finding = `<p class="title">${esc(r.title)}</p><p>${esc(r.notes)}</p>${meta}`;
  } else if (r.stage === 'assessing') {
    finding = '<p class="muted">The desk is assessing this claim: it finds the vehicle in each photo, assesses the damage and checks every photo against earlier claims.</p>';
  } else if (r.stage === 'worked') {
    finding = '<p class="muted">The desk ended its turn without an estimate or a referral.</p>';
  } else {
    finding = `<p class="muted">${S.day === 'waiting' ? 'Not worked yet: the desk starts when you press Start the day.' : 'In the queue: the desk works the claims in the order they arrived.'}</p>`;
  }
  return `<div class="photos">${photos}</div><dl class="facts"><dt>Policy</dt><dd>${esc(r.policy)}</dd><dt>Insured vehicle</dt><dd>${esc(r.vehicle)}</dd>`
    + `<dt>Date of loss</dt><dd>${esc(r.date)}</dd><dt>What happened</dt><dd>${esc(r.story)}</dd></dl><div class="finding">${finding}</div>`;
}

function decisionBody(r, since) {
  const e = r.estimate, f = r.referral;
  if (e) {
    const lines = (e.lines || []).map(l => `<tr><td>${esc(l.part)}</td><td>${l.action === 'replace' ? 'Replace' : 'Repair'}</td>`
      + `<td class="n">${Number(l.hours).toFixed(1)}</td><td class="n">${amount(l.amount)}</td></tr>`).join('');
    return `<div class="card-head"><b>Estimate · ${esc(e.severity)}${(e.sides || []).length ? ' · ' + esc(e.sides.join(', ')) : ''}</b>${since}</div>`
      + `<table><thead><tr><th>Part</th><th>Work</th><th class="n">Hours</th><th class="n">Amount</th></tr></thead><tbody>${lines}</tbody>`
      + `<tfoot><tr><td colspan="2">Total</td><td class="n">${Number(e.hours).toFixed(1)}</td><td class="n">${amount(e.total)}</td></tr></tfoot></table>`
      + `<p class="fine">Parts ${amount(e.parts)} and labour ${amount(e.labour)} at ${amount(e.labourRate)} an hour, from an illustrative price list that names no currency.</p>`;
  }
  if (!f) return '';
  if (f.reason === 'reused_photo') {
    const m = f.earlier;
    const pair = photo(f.photo, 'This claim · ' + f.photo, `${f.photo}, sent with this claim`)
      + (m ? photo(m.photo, `Earlier claim ${m.claim} · ${m.photo}`, `${m.photo}, from earlier claim ${m.claim}`) : '');
    const score = m ? `<p>The photos score ${Number(m.similarity).toFixed(2)} alike, where 1 is the same image; 0.95 or more marks a reused photo.</p>` : '';
    return `<div class="card-head"><b>Referral to investigations · reused photo</b>${since}</div><div class="pair">${pair}</div>${score}`;
  }
  const pair = photo(f.photo, 'The photo in question · ' + f.photo, `${f.photo}, the photo in question`)
    + (f.other ? photo(f.other, "The claim's other photo · " + f.other, `${f.other}, the claim's other photo`) : '');
  return `<div class="card-head"><b>Referral to investigations · another vehicle</b>${since}</div><div class="pair">${pair}</div>`
    + `<p><span class="muted">Insured vehicle:</span> ${esc(r.vehicle)}</p>`;
}

function renderDecision(r) {
  const card = cards[r.id];
  $('decision-wrap').hidden = !card;
  if (!card) { paint('decision', ''); return; }
  if (declining === card.id && card.status === 'waiting') return; // keep the reason being typed
  declining = null;
  const note = decisionNote[r.id];
  const noteHtml = note ? `<p class="status-note ${note.err ? 'err' : ''}" role="status">${esc(note.text)}</p>` : '';
  if (card.status === 'waiting') {
    const since = `<span class="since">Waiting since ${esc((card.sinceText || '').replace(/^since /, ''))}</span>`;
    paint('decision', `<div class="card waiting">${decisionBody(r, since)}`
      + `<p class="fine">${r.estimate ? 'Approve accepts the estimate' : 'Approve sends the claim to the special investigations team'}; Decline sends it back to the desk with your reason.</p>`
      + `<div class="actions" id="d-actions"><button type="button" class="primary" data-act="approve">Approve</button><button type="button" data-act="decline">Decline</button></div>`
      + `<div class="decline" id="d-decline" hidden><label>Why you decline, if you want to say (the desk reads it)<textarea id="d-reason" maxlength="500"></textarea></label>`
      + `<div class="actions"><button type="button" class="primary" data-act="confirm">Confirm decline</button><button type="button" data-act="cancel">Cancel</button></div></div>${noteHtml}</div>`);
    return;
  }
  const d = card.decided || {};
  const verb = d.answer === 'yes' ? 'approved' : d.answer === 'no' ? 'declined' : 'closed this';
  const reason = d.answer === 'no' && r.declineReason ? ': ' + r.declineReason : '';
  paint('decision', `<div class="card"><p class="decided ${d.answer === 'no' ? 'no' : ''}">${esc((d.byYou ? 'You' : d.by || 'Someone') + ' ' + verb + ' at ' + (d.atText || '') + reason)}</p>`
    + `<details><summary>${r.estimate ? 'The estimate you decided on' : 'The referral you decided on'}</summary>${decisionBody(r, '')}</details>${noteHtml}</div>`);
}

// A turn starts at each message the person (or the day, for the claim's first) sent; only the
// last text of a finished turn is the desk's reply, and tool steps fold into one line.
function turnsOf(entries, task) {
  const turns = [];
  let t = null, prev = null, skip = false;
  for (const e of entries) {
    const at = Date.parse(e.at);
    for (const it of e.items) {
      if (it.kind === 'message' && it.from === 'you') {
        t = {ask: it.text, arrived: it.text === task, atText: e.atText, steps: 0, ms: 0, reply: null, held: false};
        turns.push(t); prev = at; skip = false; continue;
      }
      if (!t) { t = {ask: null, atText: e.atText, steps: 0, ms: 0, reply: null, held: false}; turns.push(t); prev = at; }
      if (!skip && prev !== null) t.ms += Math.max(0, at - prev);
      prev = at;
      skip = it.kind === 'decision'; // the wait for the person is not the desk's work
      if (it.kind === 'activity') t.steps++;
      if (skip) t.held = true;
      if (it.kind === 'message') t.reply = {text: it.text, atText: e.atText};
    }
  }
  return turns;
}
function conversationHtml(r) {
  if (!thread || !thread.entries.length) {
    return `<p class="muted">${r.stage === 'new' ? 'Nothing yet: the conversation starts when the desk picks up this claim.' : 'The desk is opening this claim…'}</p>`;
  }
  const running = ['assessing', 'answering', 'waiting'].includes(r.turn.state);
  const turns = turnsOf(thread.entries, r.task);
  return turns.map((t, i) => {
    let h = '';
    if (t.arrived) h += `<p class="sys">${esc(t.atText)} · The claim arrived with ${plural(r.photos.length, 'photo', 'photos')} and the desk started on it</p>`;
    else if (t.ask) h += `<div class="msg you"><span class="who">You · ${esc(t.atText)}</span>${esc(t.ask)}</div>`;
    if (t.steps) h += `<p class="steps">Worked for ${dur(t.ms)} · ${plural(t.steps, 'step', 'steps')}${t.held ? ' · held for your decision' : ''}</p>`;
    if (t.reply && (i < turns.length - 1 || !running)) h += `<div class="msg agent"><span class="who">Claims Desk · ${esc(t.reply.atText)}</span>${esc(t.reply.text)}</div>`;
    return `<div class="turn">${h}</div>`;
  }).join('');
}

function renderAsk(r) {
  const live = S.mode === 'live';
  const input = $('ask-input');
  input.placeholder = `Ask about ${r.id}…`;
  input.disabled = !live;
  $('ask-send').disabled = !live || sending;
  paint('ask-note', live ? '' : 'Questions need a live run: set ANTHROPIC_API_KEY and start with <code>serve --live</code>.');
  const runs = r.turn.runs, wait = awaiting[r.id], note = askNote[r.id];
  if (note && note.until !== undefined && runs > note.until) delete askNote[r.id];
  if (wait !== undefined && runs > wait && r.turn.state !== 'answering') delete awaiting[r.id];
  let text = '', err = false;
  if (askNote[r.id]) ({text, err} = askNote[r.id]);
  else if (r.turn.state === 'answering' || awaiting[r.id] !== undefined) text = 'Claims Desk is answering…';
  else if (r.turn.state === 'assessing') text = 'Claims Desk is assessing this claim…';
  else if (r.turn.state === 'failed') { text = 'Claude could not answer: ' + (r.turn.error || 'the turn failed and gave no reason'); err = true; }
  else if (r.turn.state === 'waiting' && cards[r.id] && cards[r.id].status === 'waiting') text = 'The desk waits for your decision above.';
  $('ask-status').className = 'status-note' + (err ? ' err' : '');
  paint('ask-status', esc(text));
}

function renderClaim() {
  const r = chosen && row(chosen);
  $('empty').hidden = !!r;
  $('claim').hidden = !r;
  if (!r) return;
  const [status, cls] = statusOf(r);
  paint('c-title', esc(r.id + ' · ' + r.vehicle));
  $('c-status').className = 'tag ' + cls;
  $('c-status').textContent = status;
  $('conv-h').textContent = 'Conversation about ' + r.id;
  paint('found', foundHtml(r));
  renderDecision(r);
  paint('conversation', conversationHtml(r));
  renderAsk(r);
}

async function refresh() {
  // While the desk is unreachable, ask every 5 s rather than every second.
  if (offlineSince && Date.now() - offlineSince < 5000) return;
  try {
    const [state, decisions] = await Promise.all([getJson('/claims/state'),
      getJson('/api/surface/decisions?surface=web&decidedSince=2000-01-01T00:00:00Z')]);
    S = state;
    cards = byClaim(decisions.decisions);
    const r = chosen && row(chosen);
    thread = r && r.stage !== 'new' ? await getJson(`/api/surface/sessions/${enc(r.id)}/thread?surface=web`) : null;
    $('offline').hidden = true;
    offlineSince = 0;
  } catch (e) {
    $('offline').hidden = false;
    offlineSince = Date.now();
    return;
  }
  renderHeader();
  renderList();
  renderClaim();
}
function tick() { if (!ticking) ticking = refresh().finally(() => { ticking = null; }); return ticking; }

function choose(id) {
  if (chosen !== id) {
    chosen = id; declining = null; thread = null;
    delete painted.decision; delete painted.conversation;
    $('ask-input').value = '';
    history.replaceState(null, '', '#' + enc(id));
  }
  if (S) { renderList(); renderClaim(); }
  // On a phone the claim sits below the list.
  if (window.matchMedia('(max-width: 720px)').matches) $('claim').scrollIntoView({block: 'start'});
  tick();
}
window.addEventListener('hashchange', () => { const id = decodeURIComponent(location.hash.slice(1)); if (id) choose(id); });
$('list').addEventListener('click', e => { const b = e.target.closest('button[data-id]'); if (b) choose(b.dataset.id); });
$('start').addEventListener('click', async () => {
  $('start').disabled = true;
  await fetch('/claims/start', {method: 'POST'}).catch(() => null);
  tick();
});
$('decision').addEventListener('click', async e => {
  const button = e.target.closest('button[data-act]');
  const card = cards[chosen];
  if (!button || !card || card.status !== 'waiting') return;
  const act = button.dataset.act;
  if (act === 'decline') { declining = card.id; $('d-actions').hidden = true; $('d-decline').hidden = false; $('d-reason').focus(); return; }
  if (act === 'cancel') { declining = null; delete painted.decision; renderClaim(); return; }
  const id = chosen, answer = act === 'approve' ? 'allowOnce' : 'deny';
  const cmd = {type: 'decide', decisionId: card.id, version: card.version, answer, expanded: true};
  if (answer === 'allowOnce') cmd.evidence = ((card.media && card.media.images) || []).map(i => i.digest);
  const reason = act === 'confirm' ? $('d-reason').value.trim() : '';
  if (reason) cmd.reason = reason;
  $('decision').querySelectorAll('button').forEach(b => { b.disabled = true; });
  const res = await command(cmd);
  declining = null;
  if (res.status === 409 || res.body.type === 'changed') decisionNote[id] = {text: 'This decision changed since you read it: here it is again.', err: true};
  else if (!res.ok) decisionNote[id] = {text: 'Your decision was not recorded: ' + problem(res), err: true};
  else delete decisionNote[id];
  delete painted.decision;
  tick();
});
$('ask').addEventListener('submit', async e => {
  e.preventDefault();
  const r = row(chosen), text = $('ask-input').value.trim();
  if (!r || !text || S.mode !== 'live' || sending) return;
  sending = true;
  askNote[r.id] = {text: 'Sending…'};
  renderAsk(r);
  const res = await command({type: 'send', sessionId: r.id, text});
  sending = false;
  if (!res.ok) askNote[r.id] = {text: 'Your question was not sent: ' + problem(res), err: true};
  else {
    $('ask-input').value = '';
    awaiting[r.id] = r.turn.runs;
    if (res.body.type === 'queued') askNote[r.id] = {text: res.body.text, until: r.turn.runs};
    else delete askNote[r.id];
  }
  tick();
});
tick();
setInterval(tick, 1000);
</script>
</body>
</html>
"##;
