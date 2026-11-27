//! The AP Desk page, the whole product in the browser: the clerk starts the day, follows each
//! invoice from the inbox to the books, decides each exception beside its page and the ERP
//! mismatch, and asks about any invoice. The web home stays at `/ui/` as the developer view.

use std::collections::BTreeMap;
use std::future::IntoFuture;
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adk_artifact::ArtifactService;
use adk_core::{
    ApprovalRequest, ApprovalState, InboxQuery, RequestContext, RunState, SingleAgentLoader,
};
use adk_kernel::{ProcessFilter, ProcessKind};
use adk_server::auth_bridge::{RequestContextError, RequestContextExtractor};
use adk_server::{ServerBuilder, ServerConfig};
use adk_surface::{CardHint, CardHints, LocalSource, Place, SurfaceEngine};
use async_trait::async_trait;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::data::{NON_PO_LIMIT, PO_TOLERANCE};
use crate::desk::{APP, USER};
use crate::eval::Outcome;
use crate::tape::Mode;
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

/// The heading an exception card shows in the developer view.
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

/// Where the day is: it starts when the clerk presses **Start the day**, not when the
/// process starts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Day {
    Ready,
    Running,
    Finished,
}

impl Day {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Finished => "finished",
        }
    }
}

/// What the page's routes share.
struct Served {
    app: Arc<App>,
    day: Mutex<Day>,
    start: Notify,
    /// The invoices the day works: the inbox, or its first `--docs`.
    total: usize,
    daily_usd: f64,
}

pub async fn serve(common: Common, port: u16) -> anyhow::Result<()> {
    let app = Arc::new(App::build(&common).await?);
    app.tape.answer_agent_misses();
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
    let inbox = app.books.expected.len();
    let served = Arc::new(Served {
        app: Arc::clone(&app),
        day: Mutex::new(Day::Ready),
        start: Notify::new(),
        total: common.docs.unwrap_or(inbox).min(inbox),
        daily_usd: common.daily_usd,
    });
    let page = axum::Router::new()
        .route("/ap", get(|| async { Html(PAGE) }))
        .route("/ap/state", get(state))
        .route("/ap/start", post(start))
        .route("/ap/doc/{doc}", get(detail))
        .route("/ap/page/{doc}", get(page_image))
        .route("/ap/crop/{doc}", get(crop))
        .with_state(Arc::clone(&served));
    let router = ServerBuilder::new(config)
        .add_api_routes(adk_surface::web::router(engine))
        .add_root_routes(adk_surface::web::home().merge(page))
        .build();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("AP Desk on http://127.0.0.1:{port}/ap: press Start the day there");
    println!("developer view (the web home) on http://127.0.0.1:{port}/ui/");
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
        let served = Arc::clone(&served);
        let limit = common.docs;
        tokio::spawn(async move {
            served.start.notified().await;
            println!("[inbox] the day started");
            if let Err(err) = served.app.work(limit).await {
                eprintln!("[inbox] stopped: {err}");
            }
            *served.day.lock().unwrap_or_else(|e| e.into_inner()) = Day::Finished;
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

async fn start(State(served): State<Arc<Served>>) -> impl IntoResponse {
    let mut day = served.day.lock().unwrap_or_else(|e| e.into_inner());
    if *day == Day::Ready {
        *day = Day::Running;
        served.start.notify_one();
    }
    axum::Json(json!({ "day": day.as_str() }))
}

fn known(app: &App, doc: &str) -> bool {
    app.books.expected.iter().any(|e| e.doc == doc)
}

async fn page_image(State(served): State<Arc<Served>>, Path(doc): Path<String>) -> Response {
    let doc = doc.trim_end_matches(".jpg");
    if !known(&served.app, doc) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match std::fs::read(data::inbox_dir(&crate::root()).join(format!("{doc}.jpg"))) {
        Ok(bytes) => ([(header::CONTENT_TYPE, "image/jpeg")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct CropQuery {
    /// `x0,y0,x1,y1` in the page's pixels.
    #[serde(rename = "box")]
    region: String,
}

/// One line of a page, cut out with a margin, as PNG: the evidence an exception points at.
async fn crop(
    State(served): State<Arc<Served>>,
    Path(doc): Path<String>,
    Query(query): Query<CropQuery>,
) -> Response {
    let corners: Vec<f64> = query
        .region
        .split(',')
        .filter_map(|n| n.trim().parse().ok())
        .collect();
    let Ok([x0, y0, x1, y1]) = <[f64; 4]>::try_from(corners.as_slice()) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if !known(&served.app, &doc) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(bytes) = std::fs::read(data::inbox_dir(&crate::root()).join(format!("{doc}.jpg")))
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(page) = image::load_from_memory(&bytes) else {
        return StatusCode::UNPROCESSABLE_ENTITY.into_response();
    };
    // Wide enough for the line's label, shallow enough to leave its neighbours out.
    const SIDE: f64 = 24.0;
    const ABOVE: f64 = 6.0;
    let (w, h) = (f64::from(page.width()), f64::from(page.height()));
    let left = (x0.min(x1) - SIDE).clamp(0.0, w - 1.0);
    let top = (y0.min(y1) - ABOVE).clamp(0.0, h - 1.0);
    let right = (x0.max(x1) + SIDE).clamp(left + 1.0, w);
    let bottom = (y0.max(y1) + ABOVE).clamp(top + 1.0, h);
    let cut = page.crop_imm(
        left as u32,
        top as u32,
        (right - left) as u32,
        (bottom - top) as u32,
    );
    let mut png = Cursor::new(Vec::new());
    match cut.write_to(&mut png, image::ImageFormat::Png) {
        Ok(()) => ([(header::CONTENT_TYPE, "image/png")], png.into_inner()).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// The latest turn of each session: its state, its error and whether the clerk asked it.
async fn turns(app: &App) -> BTreeMap<String, (RunState, Option<String>, bool)> {
    let mut turns = BTreeMap::new();
    // Processes come in order of creation, so the latest of a session wins.
    for process in app
        .desk
        .kernel
        .processes()
        .ps(ProcessFilter::default())
        .await
        .unwrap_or_default()
    {
        if let Some(session) = process.run.session_id.clone() {
            let asked = matches!(process.meta.kind, ProcessKind::Interactive);
            turns.insert(
                session,
                (process.run.state, process.run.error.clone(), asked),
            );
        }
    }
    turns
}

/// The latest exception held for each invoice, decided or not.
async fn held(app: &App) -> BTreeMap<String, ApprovalRequest> {
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
    let mut held = BTreeMap::new();
    if let Ok(page) = app.desk.approvals.inbox(query).await {
        for request in page.items {
            if request.tool_name == "raise_exception" {
                held.insert(request.scope.session_id.clone(), request);
            }
        }
    }
    held
}

/// Where an invoice is, in the page's words: `inbox`, `reading`, `posted`, `waiting` (an
/// exception for the clerk), `approved`, `declined`, `ended` or `failed`.
fn row_state(
    outcome: &Outcome,
    started: bool,
    held: Option<&ApprovalRequest>,
    turn: Option<&(RunState, Option<String>, bool)>,
) -> &'static str {
    match (outcome.decision.as_deref(), held.map(|r| r.state)) {
        (Some("post"), _) => "posted",
        (Some(_), Some(ApprovalState::Approved)) => "approved",
        (Some(_), Some(ApprovalState::Rejected)) => "declined",
        (
            Some(_),
            Some(ApprovalState::Expired | ApprovalState::Cancelled | ApprovalState::Responded),
        ) => "ended",
        (Some(_), Some(ApprovalState::Pending) | None) => "waiting",
        (None, _) if !started => "inbox",
        (None, _) if turn.is_some_and(|(state, _, _)| *state == RunState::Failed) => "failed",
        (None, _) => "reading",
    }
}

async fn state(State(served): State<Arc<Served>>) -> impl IntoResponse {
    let app = &served.app;
    let started = app
        .started
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let held = held(app).await;
    let turns = turns(app).await;
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut rows = Vec::new();
    for expected in app.books.expected.iter().take(served.total) {
        let doc = &expected.doc;
        let begun = started.contains(doc);
        let calls = if begun {
            app.desk.calls(doc).await
        } else {
            Vec::new()
        };
        let outcome = Outcome::from_calls(doc, &calls);
        let state = row_state(&outcome, begun, held.get(doc), turns.get(doc));
        *counts.entry(state).or_default() += 1;
        let fields = outcome.fields.clone().unwrap_or(Value::Null);
        // While a page is read, the supplier the agent looked up names the row.
        let supplier = match &fields["supplier"] {
            Value::String(name) => json!(name),
            _ => calls
                .iter()
                .rev()
                .find(|(name, _)| name == "erp_lookup")
                .map_or(Value::Null, |(_, args)| args["supplier"].clone()),
        };
        rows.push(json!({
            "doc": doc,
            "state": state,
            "kind": outcome.decision.as_deref().filter(|d| *d != "post"),
            "supplier": supplier,
            "number": fields["number"],
            "total": fields["total"],
            "currency": fields["currency"],
        }));
    }
    let count = |state: &str| counts.get(state).copied().unwrap_or_default();
    let spend: f64 = app.tape.spend().values().map(|s| s.usd).sum();
    let day = *served.day.lock().unwrap_or_else(|e| e.into_inner());
    axum::Json(json!({
        "mode": if app.tape.mode == Mode::Live { "live" } else { "replay" },
        "day": day.as_str(),
        "total": served.total,
        "worked": served.total - count("inbox") - count("reading") - count("failed"),
        "posted": count("posted"),
        "waiting": count("waiting"),
        "decided": count("approved") + count("declined"),
        // `abs` turns the empty sum's -0.0 into 0.
        "spendUsd": ((spend * 100.0).round() / 100.0).abs(),
        "dailyUsd": served.daily_usd,
        "documents": rows,
    }))
}

/// One invoice for the detail panel: what was read, the match against the ERP, the exception
/// with its mismatch and evidence, the clerk's decision and the latest turn.
async fn detail(State(served): State<Arc<Served>>, Path(doc): Path<String>) -> Response {
    let app = &served.app;
    if !known(app, &doc) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let started = app
        .started
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&doc);
    let (calls, answers) = if started {
        (app.desk.calls(&doc).await, app.desk.answers(&doc).await)
    } else {
        (Vec::new(), Vec::new())
    };
    let outcome = Outcome::from_calls(&doc, &calls);
    let held = held(app).await.remove(&doc);
    let turn = turns(app).await.remove(&doc);
    let state = row_state(&outcome, started, held.as_ref(), turn.as_ref());
    let answer = |tool: &str| {
        answers
            .iter()
            .rev()
            .find(|(name, _)| name == tool)
            .map(|(_, response)| response)
    };
    let fields = outcome.fields.clone().unwrap_or(Value::Null);
    let erp = answer("erp_lookup");
    let similar = answer("similar_invoices");
    let exception = calls
        .iter()
        .rev()
        .find(|(name, _)| name == "raise_exception")
        .map(|(_, args)| {
            let kind = args["kind"].as_str().unwrap_or_default();
            json!({
                "kind": kind,
                "title": args["title"],
                "detail": args["detail"],
                "mismatch": mismatch(kind, &fields, erp, similar, app)
                    .unwrap_or_else(|| args["title"].as_str().unwrap_or_default().to_string()),
                "crop": evidence_line(kind, &fields, answer("vision_read")),
            })
        });
    let decision = held.map(|request| {
        let last = request.decisions.last();
        json!({
            "state": match request.state {
                ApprovalState::Pending => "waiting",
                ApprovalState::Approved => "approved",
                ApprovalState::Rejected => "declined",
                ApprovalState::Responded | ApprovalState::Expired | ApprovalState::Cancelled => "ended",
            },
            "at": last.map(|d| d.decided_at).or(request.resolved_at),
            "reason": last.and_then(|d| d.reason.clone()),
        })
    });
    let turn = turn.map(|(run, error, asked)| {
        json!({
            "state": match run {
                RunState::Queued | RunState::Running => "running",
                RunState::Waiting => "waiting",
                RunState::Completed => "done",
                RunState::Failed => "failed",
                RunState::Cancelled => "stopped",
                // `RunState` is non-exhaustive: a state added later reads as still at work.
                _ => "running",
            },
            "error": error.as_deref().map(plain_error),
            "asked": asked,
        })
    });
    axum::Json(json!({
        "doc": doc,
        "started": started,
        "state": state,
        "read": outcome.fields,
        "crops": outcome.crops,
        "posting": app.desk.ledger.entry(&doc).and_then(|e| e["posting"].as_str().map(str::to_string)),
        "match": erp.map(|erp| match_rows(&fields, erp, similar, app)).unwrap_or_default(),
        "exception": exception,
        "decision": decision,
        "turn": turn,
    }))
    .into_response()
}

/// A failed turn's error in words a clerk can act on: the error code and the provider's type
/// tag dropped, a refused key or a rate limit said plainly.
fn plain_error(error: &str) -> String {
    let (code, said) = error
        .split_once(": ")
        .filter(|(code, _)| code.contains('.') && !code.contains(' '))
        .unwrap_or(("", error));
    let said = said.split(" [type=").next().unwrap_or(said).trim();
    let said: String = said.chars().take(300).collect();
    match code.rsplit('.').next() {
        Some("unauthorized") => {
            format!("the API key was refused; check ANTHROPIC_API_KEY ({said})")
        }
        Some("rate_limited") => {
            format!("the API is limiting requests from this key; ask again in a minute ({said})")
        }
        _ => said,
    }
}

fn alnum(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// An amount as an AP clerk reads it: `1,171.68`.
fn money(value: f64) -> String {
    let cents = (value.abs() * 100.0).round() as u64;
    let whole = (cents / 100).to_string();
    let mut grouped = String::new();
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let sign = if value < 0.0 { "-" } else { "" };
    format!("{sign}{grouped}.{:02}", cents % 100)
}

/// The supplier's earlier posting with the same invoice number, from the agent's ERP lookup:
/// its document and posting id.
fn posted_twin(fields: &Value, erp: &Value, app: &App) -> Option<(String, String)> {
    let number = alnum(fields["number"].as_str()?);
    let twin = erp["posted"]
        .as_array()?
        .iter()
        .find(|p| !number.is_empty() && p["number"].as_str().is_some_and(|n| alnum(n) == number))?;
    let doc = twin["doc"].as_str()?.to_string();
    let posting = app
        .desk
        .ledger
        .entry(&doc)
        .and_then(|e| e["posting"].as_str().map(str::to_string))
        .unwrap_or_else(|| "posted".into());
    Some((doc, posting))
}

/// The most alike earlier page, and whether it has the same supplier, total and date.
fn look_alike(fields: &Value, similar: Option<&Value>) -> Option<(String, f64, bool)> {
    let hit = similar?["similar"].as_array()?.first()?;
    let theirs = &hit["desk"]["fields"];
    let same = alnum(theirs["supplier"].as_str().unwrap_or_default())
        == alnum(fields["supplier"].as_str().unwrap_or("-"))
        && theirs["total"]
            .as_f64()
            .zip(fields["total"].as_f64())
            .is_some_and(|(a, b)| (a - b).abs() < 0.005)
        && theirs["date"] == fields["date"];
    Some((
        hit["doc"].as_str()?.to_string(),
        hit["similarity"].as_f64()?,
        same,
    ))
}

/// The exception's mismatch against the PO and the ERP, in plain words.
fn mismatch(
    kind: &str,
    fields: &Value,
    erp: Option<&Value>,
    similar: Option<&Value>,
    app: &App,
) -> Option<String> {
    let total = fields["total"].as_f64();
    let currency = fields["currency"].as_str().unwrap_or_default();
    let po = erp.map(|erp| &erp["purchaseOrder"]);
    match kind {
        "over_po" => {
            let po = po?;
            let (total, amount) = (total?, po["amount"].as_f64()?);
            let over = total - amount;
            Some(format!(
                "The invoice total of {} {currency} is {} ({:.1}%) above PO {}'s {} {currency}; the tolerance is {:.0}%.",
                money(total),
                money(over),
                over / amount * 100.0,
                po["number"].as_str().unwrap_or_default(),
                money(amount),
                PO_TOLERANCE * 100.0
            ))
        }
        "not_received" => {
            let po = po?;
            Some(format!(
                "PO {} for {} {} has no goods receipt in the ERP: nothing has been received against it yet.",
                po["number"].as_str().unwrap_or_default(),
                money(po["amount"].as_f64()?),
                po["currency"].as_str().unwrap_or(currency)
            ))
        }
        "over_limit" => {
            let total = total?;
            Some(format!(
                "No PO is printed, and the total of {} {currency} is {} above the {} limit for invoices without one.",
                money(total),
                money(total - NON_PO_LIMIT),
                money(NON_PO_LIMIT).trim_end_matches(".00")
            ))
        }
        "unknown_supplier" => Some(format!(
            "The ERP has no approved supplier named \u{201c}{}\u{201d}.",
            fields["supplier"].as_str()?
        )),
        "duplicate" => {
            if let Some((doc, posting)) = erp.and_then(|erp| posted_twin(fields, erp, app)) {
                return Some(format!(
                    "Invoice {} from {} was already posted as {doc} ({posting}).",
                    fields["number"].as_str().unwrap_or_default(),
                    fields["supplier"].as_str().unwrap_or_default()
                ));
            }
            let (doc, score, same) = look_alike(fields, similar)?;
            same.then(|| {
                format!(
                    "The page matches {doc} at {score:.2} similarity, with the same supplier, total and date."
                )
            })
        }
        "unreadable" => Some(
            "AP Desk could not read every field it needs from this page with confidence.".into(),
        ),
        _ => None,
    }
}

/// The match against the ERP, row by row: label, value and tone (`ok`, `warn` or `plain`).
fn match_rows(fields: &Value, erp: &Value, similar: Option<&Value>, app: &App) -> Vec<Value> {
    let row = |label: &str, value: String, tone: &str| json!({ "label": label, "value": value, "tone": tone });
    let mut rows = Vec::new();
    let supplier = &erp["supplier"];
    rows.push(if supplier["approved"].as_bool() == Some(true) {
        row(
            "Supplier",
            format!(
                "Approved, {}",
                supplier["vendorId"].as_str().unwrap_or_default()
            ),
            "ok",
        )
    } else {
        row("Supplier", "Not an approved supplier".into(), "warn")
    });
    let total = fields["total"].as_f64();
    let po = &erp["purchaseOrder"];
    match po["amount"].as_f64() {
        _ if po.is_null() => {
            rows.push(row("Purchase order", "None printed".into(), "plain"));
            if let Some(total) = total {
                rows.push(if total > NON_PO_LIMIT {
                    row(
                        "Limit without a PO",
                        format!("{} over the 1,000 limit", money(total - NON_PO_LIMIT)),
                        "warn",
                    )
                } else {
                    row("Limit without a PO", "Under the 1,000 limit".into(), "ok")
                });
            }
        }
        None => rows.push(row(
            "Purchase order",
            format!(
                "PO {} is not in the ERP for this supplier",
                po["number"].as_str().unwrap_or_default()
            ),
            "warn",
        )),
        Some(amount) => {
            rows.push(row(
                "Purchase order",
                format!(
                    "PO {} for {} {}",
                    po["number"].as_str().unwrap_or_default(),
                    money(amount),
                    po["currency"].as_str().unwrap_or_default()
                ),
                "plain",
            ));
            rows.push(match po["goodsReceipt"].as_str() {
                Some(receipt) => row("Goods receipt", receipt.to_string(), "ok"),
                None => row("Goods receipt", "None recorded".into(), "warn"),
            });
            if let Some(total) = total {
                let over = (total - amount) / amount;
                rows.push(if over > PO_TOLERANCE {
                    row(
                        "Against the PO",
                        format!(
                            "{} ({:.1}%) over; the tolerance is 2%",
                            money(total - amount),
                            over * 100.0
                        ),
                        "warn",
                    )
                } else {
                    row("Against the PO", "Within the 2% tolerance".into(), "ok")
                });
            }
        }
    }
    rows.push(match posted_twin(fields, erp, app) {
        Some((doc, posting)) => row(
            "Already posted",
            format!("Same number as {doc} ({posting})"),
            "warn",
        ),
        None => row(
            "Already posted",
            "No earlier invoice with this number".into(),
            "ok",
        ),
    });
    rows.push(match look_alike(fields, similar) {
        Some((doc, score, true)) if score >= 0.92 => row(
            "Look-alike page",
            format!("{doc} at {score:.2}, same supplier, total and date"),
            "warn",
        ),
        Some((doc, score, _)) => row(
            "Look-alike page",
            format!("{doc} at {score:.2}, a different invoice"),
            "plain",
        ),
        None => row(
            "Look-alike page",
            "None among the pages seen so far".into(),
            "plain",
        ),
    });
    rows
}

/// The line of the page an exception is about, from what PP-OCRv6 read: its box and what it
/// shows. The total for an amount over the PO or the limit, the invoice number for a
/// duplicate, the PO for goods not received, the supplier otherwise.
fn evidence_line(kind: &str, fields: &Value, read: Option<&Value>) -> Option<Value> {
    let read = read?;
    let texts: BTreeMap<u64, &str> = read["content"]
        .as_str()?
        .lines()
        .filter_map(|line| {
            let (number, text) = line.strip_prefix('[')?.split_once("] ")?;
            Some((number.parse().ok()?, text))
        })
        .collect();
    let lines: Vec<(&str, [f64; 4])> = read["blocks"]
        .as_array()?
        .iter()
        .filter_map(|block| {
            let text = texts.get(&block["line"].as_u64()?)?;
            let corners: Vec<f64> = block["box"]
                .as_array()?
                .iter()
                .filter_map(Value::as_f64)
                .collect();
            Some((*text, <[f64; 4]>::try_from(corners.as_slice()).ok()?))
        })
        .collect();
    let (label, found) = match kind {
        "over_po" | "over_limit" => {
            let total = format!("{:.2}", fields["total"].as_f64()?);
            let found = lines
                .iter()
                .rev()
                .find(|(text, _)| text.replace([',', ' '], "").contains(&total));
            ("The total as printed", found)
        }
        "duplicate" => {
            let number = alnum(fields["number"].as_str()?);
            let found = lines
                .iter()
                .find(|(text, _)| !number.is_empty() && alnum(text).contains(&number));
            ("The invoice number as printed", found)
        }
        "not_received" => {
            let po = alnum(fields["po"].as_str()?);
            let found = lines.iter().find(|(text, _)| {
                let text = alnum(text);
                !po.is_empty() && text.contains("po") && text.ends_with(&po)
            });
            ("The PO number as printed", found)
        }
        _ => {
            let supplier = alnum(fields["supplier"].as_str()?);
            let found = lines
                .iter()
                .find(|(text, _)| !supplier.is_empty() && alnum(text).contains(&supplier));
            ("The supplier as printed", found)
        }
    };
    let (text, mut corners) = *found?;
    // A bare amount is printed apart from its label: the crop takes the whole printed row.
    if !text.chars().any(char::is_alphabetic) {
        let (top, bottom) = (corners[1], corners[3]);
        for (_, other) in &lines {
            let middle = (other[1] + other[3]) / 2.0;
            if middle >= top && middle <= bottom {
                corners = [
                    corners[0].min(other[0]),
                    corners[1].min(other[1]),
                    corners[2].max(other[2]),
                    corners[3].max(other[3]),
                ];
            }
        }
    }
    Some(json!({
        "box": corners.iter().map(|c| c.round()).collect::<Vec<_>>(),
        "label": label,
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
:root { color-scheme: light dark;
  --bg:#f6f5f2; --card:#fff; --raised:#f1f0ec; --border:#e4e2dc; --text:#18181b; --text2:#46464e; --label:#6b6b74;
  --btn:#c2410c; --btnText:#fff; --btnHover:#a8380a; --alert:#b42318; --alertBg:#fdecea; --ok:#1e6b45; --okBg:#e6f3eb;
  --info:#1d4ed8; --infoBg:#e8eefc; --warn:#9a5b00; --warnBg:#fdf1dc; --focus:#2563eb; }
@media (prefers-color-scheme: dark) { :root { --bg:#0e0f11; --card:#16171a; --raised:#1e1f23; --border:#2a2b31; --text:#ededef;
  --text2:#b9b9c1; --label:#8e8e97; --btn:#ff6a1f; --btnText:#0e0f11; --btnHover:#ff8547; --alert:#ff8a80; --alertBg:#3a1d1b;
  --ok:#7ad3a0; --okBg:#17301f; --info:#8fb2ff; --infoBg:#1a2440; --warn:#f5b955; --warnBg:#352812; --focus:#8fb2ff; } }
* { box-sizing: border-box; }
[hidden] { display:none !important; }
body { margin:0; font-family: Inter, system-ui, sans-serif; background: var(--bg); color: var(--text); font-size:14px; line-height:1.45; }
button, textarea { font: inherit; color: inherit; }
:focus-visible { outline: 2px solid var(--focus); outline-offset: 2px; }
.sr { position:absolute; width:1px; height:1px; overflow:hidden; clip:rect(0 0 0 0); }
header { display:grid; grid-template-columns: minmax(0,1fr) auto auto; gap:12px 28px; align-items:center; padding:14px 28px; border-bottom:1px solid var(--border); background:var(--card); }
.brand { display:flex; flex-wrap:wrap; align-items:center; gap:6px 10px; min-width:0; }
.brand .logo { width:28px; height:28px; border-radius:7px; background:#f74c00; flex:none; }
.brand h1 { font-size:17px; margin:0; }
.badge { font-size:11px; font-weight:700; letter-spacing:.02em; padding:3px 9px; border-radius:999px; }
.badge.replay { background:var(--infoBg); color:var(--info); } .badge.live { background:var(--okBg); color:var(--ok); }
.mode-line { flex-basis:100%; margin:0; color:var(--text2); font-size:13px; }
.mode-line .spend { color:var(--label); white-space:nowrap; }
.progress { min-width:220px; }
.progress b { font-size:15px; }
.bar { height:6px; border-radius:3px; background:var(--raised); margin:6px 0 4px; overflow:hidden; }
.bar i { display:block; height:100%; width:0; background:var(--ok); transition: width .4s; }
.counts { font-size:12px; color:var(--text2); display:flex; flex-wrap:wrap; gap:4px 10px; }
.counts button { border:0; background:none; padding:0; color:var(--alert); font-weight:600; cursor:pointer; text-decoration:underline; }
.day { display:flex; flex-direction:column; align-items:flex-end; gap:4px; max-width:240px; }
.day small { color:var(--label); font-size:12px; text-align:right; }
.primary { background:var(--btn); color:var(--btnText); border:0; border-radius:8px; padding:10px 18px; font-weight:600; cursor:pointer; }
.primary:hover:not(:disabled) { background:var(--btnHover); }
.secondary { background:var(--card); color:var(--text); border:1px solid var(--border); border-radius:8px; padding:9px 16px; font-weight:600; cursor:pointer; }
.secondary:hover:not(:disabled) { background:var(--raised); }
button:disabled { opacity:.55; cursor:default; }
.offline { grid-column:1 / -1; margin:0; color:var(--alert); font-size:13px; }
main { max-width:1440px; margin:0 auto; padding:24px 28px; display:flex; flex-wrap:wrap; gap:24px; align-items:flex-start; }
.inbox { flex: 1 1 340px; min-width:0; background:var(--card); border:1px solid var(--border); border-radius:12px; overflow:hidden; position:sticky; top:16px; }
.inbox h2 { margin:0; padding:14px 16px 10px; font-size:15px; display:flex; justify-content:space-between; align-items:baseline; }
.inbox h2 small { font-weight:500; color:var(--label); font-size:12px; }
.inbox ol { list-style:none; margin:0; padding:0; max-height:calc(100vh - 210px); overflow:auto; }
.inbox li { border-top:1px solid var(--border); padding:10px 16px; display:grid; grid-template-columns: 38px minmax(0,1fr) auto; gap:2px 10px; cursor:pointer; }
.inbox li:hover, .inbox li[aria-selected=true] { background:var(--raised); }
.inbox li[aria-selected=true] { box-shadow: inset 3px 0 0 var(--btn); }
.inbox .id { font-weight:700; font-size:13px; }
.inbox .who { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-weight:500; }
.inbox .who.muted { color:var(--label); font-weight:400; }
.inbox .sub { grid-column: 2 / 4; color:var(--text2); font-size:12px; }
.tag { font-size:11px; font-weight:700; padding:3px 8px; border-radius:999px; white-space:nowrap; align-self:start; }
.t-ok { background:var(--okBg); color:var(--ok); } .t-alert { background:var(--alertBg); color:var(--alert); }
.t-info { background:var(--infoBg); color:var(--info); } .t-muted { background:var(--raised); color:var(--text2); }
.t-done { background:var(--raised); color:var(--text); border:1px solid var(--border); }
.detail { flex: 2 1 600px; min-width:0; background:var(--card); border:1px solid var(--border); border-radius:12px; padding:20px 22px 22px; }
.empty { color:var(--text2); padding:28px 4px; text-align:center; }
.empty b { display:block; color:var(--text); font-size:16px; margin-bottom:6px; }
.doc-head { display:flex; flex-wrap:wrap; gap:8px 12px; align-items:center; margin-bottom:16px; }
.doc-head h2 { margin:0; font-size:18px; }
.detail h3 { font-size:14px; margin:0 0 10px; text-transform:none; }
.detail section + section, .ask { margin-top:22px; padding-top:20px; border-top:1px solid var(--border); }
.found-grid { display:grid; grid-template-columns: minmax(0, 5fr) minmax(0, 6fr); gap:20px; align-items:start; }
.page-link { display:block; border:1px solid var(--border); border-radius:6px; background:#fff; overflow:hidden; }
.page-link img { display:block; width:100%; height:auto; max-height:520px; object-fit:contain; }
dl { margin:0 0 14px; display:grid; grid-template-columns:auto minmax(0,1fr); gap:7px 14px; }
dt { color:var(--label); }
dd { margin:0; font-weight:600; overflow-wrap:anywhere; }
dd.ok { color:var(--ok); } dd.warn { color:var(--warn); }
h4 { margin:4px 0 8px; font-size:13px; }
.quiet { color:var(--text2); font-weight:400; margin:0 0 12px; }
.decision { background:var(--warnBg); border:1px solid var(--border); border-radius:10px; padding:16px 18px; }
.decision.done { background:var(--raised); }
.detail section.decision { margin-top:22px; }
.ex-kind { font-size:12px; font-weight:700; color:var(--warn); text-transform:uppercase; letter-spacing:.04em; }
.ex-title { font-size:16px; font-weight:700; margin:4px 0 8px; }
.ex-mismatch { margin:0 0 12px; font-size:14px; }
.crop { margin:0 0 12px; }
.crop img { display:block; max-width:100%; border:1px solid var(--border); border-radius:6px; background:#fff; image-rendering:auto; }
.crop figcaption { font-size:12px; color:var(--text2); margin-top:6px; }
a { color:var(--info); }
.suggest { margin:0 0 12px; padding:10px 12px; border-left:3px solid var(--border); background:var(--card); border-radius:0 6px 6px 0; }
.suggest b { display:block; font-size:12px; color:var(--label); margin-bottom:2px; }
.help { font-size:12px; color:var(--text2); margin:0 0 12px; }
.actions { display:flex; flex-wrap:wrap; gap:10px; align-items:center; }
.decline-box { margin-top:12px; display:grid; gap:8px; max-width:520px; }
.decline-box label { font-size:12px; font-weight:600; }
textarea { width:100%; border:1px solid var(--border); border-radius:8px; padding:9px 11px; background:var(--card); resize:vertical; }
.note { font-size:13px; color:var(--text2); margin:10px 0 0; min-height:1em; }
.note.err { color:var(--alert); }
details.answered summary { cursor:pointer; font-weight:600; }
details.answered[open] summary { margin-bottom:10px; }
.thread { list-style:none; margin:0; padding:0; display:grid; gap:10px; }
.thread .sys, .thread .work { color:var(--label); font-size:12px; }
.thread .work::before { content:"\2022  "; }
.thread .msg { padding:10px 12px; border-radius:10px; max-width:640px; }
.thread .msg .by { font-size:12px; color:var(--label); margin-bottom:2px; }
.thread .msg p { margin:0; white-space:pre-wrap; overflow-wrap:anywhere; }
.thread .you { background:var(--infoBg); justify-self:end; }
.thread .agent { background:var(--raised); }
.thread .err { color:var(--alert); background:var(--alertBg); padding:8px 12px; border-radius:8px; }
.ask form { display:grid; grid-template-columns: minmax(0,1fr) auto; gap:10px; align-items:end; }
.ask code { font-size:12px; background:var(--raised); padding:1px 5px; border-radius:4px; }
footer { padding:14px 28px 22px; font-size:12px; color:var(--label); display:flex; flex-wrap:wrap; gap:6px 20px; justify-content:space-between; max-width:1440px; margin:0 auto; }
footer a { color:var(--text2); }
@media (max-width: 900px) { header { grid-template-columns: 1fr; } .day { align-items:flex-start; } .day small { text-align:left; } .inbox { position:static; } .inbox ol { max-height:50vh; } }
@media (max-width: 640px) { main { padding:16px; } header { padding:14px 16px; } .detail { padding:16px; } .found-grid { grid-template-columns:1fr; } footer { padding:14px 16px 20px; } }
</style>
</head>
<body>
<header>
  <div class="brand"><span class="logo" aria-hidden="true"></span><h1>AP Desk</h1><span id="mode" class="badge"></span>
    <p id="mode-line" class="mode-line"></p></div>
  <div class="progress" aria-live="polite"><b id="worked">Loading the inbox…</b><div class="bar" aria-hidden="true"><i id="bar"></i></div><div id="counts" class="counts"></div></div>
  <div class="day"><button id="start" class="primary" disabled>Start the day</button><small id="day-hint"></small></div>
  <p id="offline" class="offline" role="alert" hidden></p>
</header>
<main>
  <section class="inbox" aria-label="Inbox"><h2>Inbox <small id="inbox-note"></small></h2><ol id="list"></ol></section>
  <section class="detail" aria-label="Invoice">
    <div id="empty" class="empty"></div>
    <div id="doc" hidden>
      <div class="doc-head"><h2 id="doc-title"></h2><span id="doc-tag" class="tag"></span></div>
      <section aria-labelledby="found-h"><h3 id="found-h">What AP Desk read</h3>
        <div class="found-grid">
          <a id="page-link" class="page-link" target="_blank" rel="noopener"><img id="page" alt=""></a>
          <div><dl id="fields"></dl><h4>Against the ERP</h4><div id="match"></div></div>
        </div>
      </section>
      <section id="decision" class="decision" aria-labelledby="decision-h" hidden></section>
      <section aria-labelledby="talk-h"><h3 id="talk-h">Conversation</h3><ol id="thread" class="thread" aria-live="polite"></ol></section>
      <section class="ask" aria-label="Ask AP Desk">
        <form id="ask"><label for="ask-text" class="sr" id="ask-label">Ask AP Desk</label>
          <textarea id="ask-text" rows="2"></textarea><button id="ask-send" class="primary" type="submit">Send</button></form>
        <p id="ask-note" class="note" role="status"></p>
      </section>
    </div>
  </section>
</main>
<footer><span>Invoices: FATURA (Limam et al.), Zenodo record 8261508, CC BY 4.0; synthetic invoices with invented companies. The ERP is generated.</span><a href="/ui/">Developer view</a></footer>
<script>
const $ = id => document.getElementById(id);
const KIND = { duplicate: 'Duplicate', unknown_supplier: 'Unknown supplier', over_po: 'Over the PO', not_received: 'Goods not received', over_limit: 'Over the limit without a PO', unreadable: 'Unreadable' };
let S = null, chosen = null, D = null, card = null, cardKey = '', threadHtml = '', listHtml = '', answering = 0, askNote = null, deciding = false;
function esc(s) { return String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c])); }
function money(v, cur) { return v == null ? '' : Number(v).toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 }) + (cur ? ' ' + cur : ''); }
function usd(v) { return '$' + Number(v || 0).toFixed(2); }
function hhmm(iso) { return new Date(iso).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }); }
function took(ms) { const s = Math.max(1, Math.round(ms / 1000)); return s < 60 ? s + ' s' : Math.floor(s / 60) + ' min ' + (s % 60) + ' s'; }
async function getJson(url) { const r = await fetch(url); if (!r.ok) throw new Error('HTTP ' + r.status); return r.json(); }
function words(r) {
  const kind = r.kind ? KIND[r.kind] || r.kind : '';
  switch (r.state) {
    case 'inbox': return { tag: '', cls: '', who: 'Waiting to be read', muted: true };
    case 'reading': return { tag: 'Reading', cls: 't-info', who: r.supplier || 'Being read now', muted: !r.supplier };
    case 'posted': return { tag: 'Posted', cls: 't-ok', who: r.supplier };
    case 'waiting': return { tag: 'Your decision', cls: 't-alert', who: r.supplier, kind };
    case 'approved': return { tag: 'You approved', cls: 't-done', who: r.supplier, kind };
    case 'declined': return { tag: 'You declined', cls: 't-done', who: r.supplier, kind };
    case 'ended': return { tag: 'Closed', cls: 't-muted', who: r.supplier, kind };
    default: return { tag: 'Could not finish', cls: 't-alert', who: r.supplier || 'Stopped before a decision', muted: !r.supplier };
  }
}
function renderHeader() {
  const live = S.mode === 'live';
  $('mode').textContent = live ? 'Live' : 'Replay';
  $('mode').className = 'badge ' + (live ? 'live' : 'replay');
  $('mode-line').innerHTML = live
    ? `Claude answers as you watch <span class="spend">· ${usd(S.spendUsd)} of ${usd(S.dailyUsd)}</span>`
    : `A recorded day played back: no key, no spend. Decisions work; new questions need a live run. <span class="spend">${usd(S.spendUsd)} when it was recorded</span>`;
  $('worked').textContent = `${S.worked} of ${S.total} invoices worked`;
  $('bar').style.width = (S.total ? 100 * S.worked / S.total : 0) + '%';
  const parts = [`<span>${S.posted} posted</span>`];
  if (S.waiting) parts.push(`<button type="button" id="next-decision">${S.waiting} waiting for you</button>`);
  else parts.push('<span>none waiting for you</span>');
  if (S.decided) parts.push(`<span>${S.decided} decided by you</span>`);
  $('counts').innerHTML = parts.join('');
  const next = $('next-decision');
  if (next) next.onclick = () => { const r = S.documents.find(x => x.state === 'waiting'); if (r) choose(r.doc); };
  const start = $('start'), hint = $('day-hint');
  if (S.day === 'ready') { start.disabled = false; start.textContent = 'Start the day'; hint.textContent = 'Nothing is read until you start.'; }
  else if (S.day === 'running') { start.disabled = true; start.textContent = 'Day started'; hint.textContent = 'AP Desk is working the inbox in arrival order.'; }
  else { start.disabled = true; start.textContent = 'Day finished'; hint.textContent = 'Restart the app to run it again.'; }
  $('inbox-note').textContent = S.day === 'ready' ? `${S.total} waiting to be read` : `${S.total - S.worked} still to work`;
}
function renderList() {
  const html = S.documents.map(r => {
    const w = words(r);
    const sub = [r.total != null ? money(r.total, r.currency) : '', r.number ? 'No. ' + r.number : '', w.kind || ''].filter(Boolean).join(' · ');
    return `<li data-doc="${esc(r.doc)}" tabindex="0" role="option" aria-selected="${r.doc === chosen}"><span class="id">${esc(r.doc)}</span><span class="who${w.muted ? ' muted' : ''}">${esc(w.who)}</span>${w.tag ? `<span class="tag ${w.cls}">${esc(w.tag)}</span>` : '<span></span>'}${sub ? `<span class="sub">${esc(sub)}</span>` : ''}</li>`;
  }).join('');
  if (html === listHtml) return;
  listHtml = html;
  const list = $('list');
  list.innerHTML = html;
  list.setAttribute('role', 'listbox');
  list.querySelectorAll('li').forEach(li => {
    li.onclick = () => choose(li.dataset.doc);
    li.onkeydown = e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); choose(li.dataset.doc); } };
  });
}
function renderEmpty() {
  $('doc').hidden = !chosen; $('empty').hidden = !!chosen;
  if (chosen) return;
  $('empty').innerHTML = S && S.day === 'ready'
    ? `<b>The inbox holds ${S.total} supplier invoices</b>Press <b style="display:inline">Start the day</b> and AP Desk reads each one, matches it against the ERP, and posts it or brings it to you. Choose an invoice to follow it.`
    : '<b>Choose an invoice</b>See what AP Desk read from it, how it matched the ERP, and decide any exception it brings to you.';
}
function choose(doc) {
  if (chosen !== doc) { chosen = doc; D = null; card = null; cardKey = ''; threadHtml = ''; askNote = null; answering = 0; $('ask-text').value = ''; $('decision').hidden = true; $('decision').innerHTML = ''; $('thread').innerHTML = ''; $('fields').innerHTML = ''; $('match').innerHTML = ''; }
  listHtml = ''; renderList(); renderEmpty(); refreshDoc();
  // On a narrow screen the invoice sits below the inbox: bring it into view.
  if (window.matchMedia('(max-width: 900px)').matches) document.querySelector('.detail').scrollIntoView({ behavior: 'smooth', block: 'start' });
}
function renderFound(d) {
  const r = S.documents.find(x => x.doc === d.doc) || { doc: d.doc, state: d.state };
  const w = words(Object.assign({}, r, { state: d.state }));
  $('doc-title').textContent = d.doc + (d.read && d.read.supplier ? ' · ' + d.read.supplier : r.supplier ? ' · ' + r.supplier : '');
  $('doc-tag').textContent = w.tag || 'In the inbox'; $('doc-tag').className = 'tag ' + (w.cls || 't-muted');
  const src = '/ap/page/' + d.doc + '.jpg', img = $('page');
  if (img.getAttribute('src') !== src) { img.src = src; img.alt = `Invoice ${d.doc} as it arrived`; $('page-link').href = src; $('page-link').title = 'Open the page at full size'; }
  let fields;
  if (d.read) {
    const f = d.read;
    const rows = [['Supplier', f.supplier], ['Invoice number', f.number], ['Date', f.date], ['PO', f.po || 'None printed'], ['Total', money(f.total, f.currency)],
      ['Crops sent to Claude', d.crops ? `${d.crops}: only the uncertain lines` : 'None: read on this computer']];
    if (d.posting) rows.push(['Posted as', d.posting]);
    fields = rows.map(([k, v]) => `<dt>${esc(k)}</dt><dd>${esc(v)}</dd>`).join('');
  } else {
    const why = d.state === 'reading' ? 'AP Desk is reading this page now.' : d.state === 'failed' ? 'AP Desk stopped before it decided; the conversation below says why.'
      : S.day === 'ready' ? 'Waiting to be read: start the day and AP Desk reads the inbox in arrival order.' : 'Waiting to be read: AP Desk reads the inbox in arrival order.';
    fields = `<dt>Fields</dt><dd style="font-weight:400">${esc(why)}</dd>`;
  }
  if ($('fields').innerHTML !== fields) $('fields').innerHTML = fields;
  const match = d.match && d.match.length
    ? '<dl>' + d.match.map(m => `<dt>${esc(m.label)}</dt><dd class="${m.tone === 'plain' ? '' : esc(m.tone)}">${esc(m.value)}</dd>`).join('') + '</dl>'
    : `<p class="quiet">${d.started ? 'Not matched yet.' : 'Matched once it is read.'}</p>`;
  if ($('match').innerHTML !== match) $('match').innerHTML = match;
}
function renderDecision(d) {
  const box = $('decision');
  if (!d.exception) { box.hidden = true; cardKey = ''; return; }
  const dec = d.decision || { state: 'waiting' };
  const key = JSON.stringify([d.doc, dec.state, dec.at, card && card.id, card && card.version, !!d.exception.crop]);
  box.hidden = false;
  if (key === cardKey) return;
  cardKey = key;
  const ex = d.exception;
  const page = card && card.media && card.media.images && card.media.images[0];
  const crop = ex.crop ? `<figure class="crop"><img src="/ap/crop/${esc(d.doc)}?box=${ex.crop.box.join(',')}" alt="${esc(ex.crop.label)} on ${esc(d.doc)}" style="width:min(100%, ${Math.round((ex.crop.box[2] - ex.crop.box[0] + 48) * 1.6)}px)"><figcaption>${esc(ex.crop.label)}${page && page.href ? ` · <a href="${esc(page.href)}" target="_blank" rel="noopener">Open the page held with this exception</a>` : ''}</figcaption></figure>`
    : page && page.href ? `<p class="help"><a href="${esc(page.href)}" target="_blank" rel="noopener">Open the page held with this exception</a></p>` : '';
  const body = `<div class="ex-kind">${esc(KIND[ex.kind] || ex.kind)}</div><p class="ex-title">${esc(ex.title)}</p><p class="ex-mismatch">${esc(ex.mismatch)}</p>${crop}<div class="suggest"><b>AP Desk suggests</b>${esc(ex.detail)}</div>`;
  if (dec.state === 'waiting') {
    box.className = 'decision';
    box.innerHTML = `<h3 id="decision-h">Your decision</h3>${body}
      <p class="help">Approve files it as an exception for follow-up. Decline tells AP Desk it should not be held, with your reason.</p>
      <div class="actions"><button type="button" class="primary" id="approve">Approve</button><button type="button" class="secondary" id="decline" aria-expanded="false" aria-controls="decline-box">Decline</button></div>
      <div class="decline-box" id="decline-box" hidden><label for="reason">Reason (optional, AP Desk reads it)</label><textarea id="reason" rows="2" placeholder="For example: the supplier sent the PO by email"></textarea><div><button type="button" class="secondary" id="confirm-decline">Confirm decline</button></div></div>
      <p class="note" id="decide-note" role="status">${card ? '' : 'Loading the decision…'}</p>`;
    $('approve').disabled = !card || deciding; $('decline').disabled = !card || deciding;
    $('approve').onclick = () => decide('allowOnce');
    $('decline').onclick = () => { const b = $('decline-box'); b.hidden = !b.hidden; $('decline').setAttribute('aria-expanded', String(!b.hidden)); if (!b.hidden) $('reason').focus(); };
    $('confirm-decline').onclick = () => decide('deny', $('reason').value.trim());
  } else {
    box.className = 'decision done';
    const at = dec.at ? ' at ' + hhmm(dec.at) : '';
    const said = dec.state === 'approved' ? `You approved${at}` : dec.state === 'declined' ? `You declined${at}${dec.reason ? ': ' + dec.reason : ''}` : `Closed without a decision${at}`;
    box.innerHTML = `<h3 id="decision-h" class="sr">Your decision</h3><details class="answered"><summary>${esc(said)}</summary>${body}</details>`;
  }
}
async function decide(answer, reason) {
  if (!card || deciding) return;
  deciding = true;
  const note = (text, err) => { const n = $('decide-note'); if (n) { n.textContent = text; n.className = 'note' + (err ? ' err' : ''); } };
  $('approve').disabled = true; $('decline').disabled = true; note('Sending your decision…');
  const command = { type: 'decide', decisionId: card.id, version: card.version, answer, expanded: true };
  if (answer === 'allowOnce' && card.media) command.evidence = card.media.images.map(i => i.digest);
  if (reason) command.reason = reason;
  try {
    const r = await fetch('/api/surface/commands', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ surface: 'web', origin: { kind: 'pointer' }, command }) });
    const body = await r.json().catch(() => ({}));
    deciding = false;
    if (r.status === 409 || body.type === 'changed') { card = null; cardKey = ''; await refreshDoc(); note('The exception changed while you looked at it; here it is as it is now.'); return; }
    if (!r.ok) { note('Your decision was not recorded: ' + (body.detail || body.title || 'HTTP ' + r.status), true); $('approve').disabled = false; $('decline').disabled = false; return; }
    card = null; cardKey = ''; await tick();
  } catch (e) { deciding = false; note('Your decision was not recorded: ' + e.message, true); $('approve').disabled = false; $('decline').disabled = false; }
}
function renderThread(d, entries) {
  const turns = []; let t = null;
  const open = (kind, text, at) => { t = { kind, text, at, steps: 0, reply: null, replyAt: null, tail: false, waited: false, busy: 0, prev: at, held: false }; turns.push(t); };
  for (const e of entries) {
    const at = Date.parse(e.at);
    for (const it of e.items) {
      if (it.kind === 'message' && it.from === 'you') {
        if (!it.text.trim()) continue;
        if (/^New invoice d\d\d in the inbox/.test(it.text)) open('arrived', '', at); else open('you', it.text, at);
        continue;
      }
      if (!t) open('agent', '', at);
      if (!t.held) t.busy += Math.max(0, at - t.prev);
      t.prev = at; t.held = false;
      if (it.kind === 'activity') { t.steps++; t.tail = true; }
      else if (it.kind === 'result' || it.kind === 'run') t.tail = true;
      else if (it.kind === 'decision') { t.waited = true; t.tail = true; t.held = true; }
      else if (it.kind === 'message' && it.from === 'agent' && it.text.trim()) { t.reply = it.text; t.replyAt = at; t.tail = false; }
    }
  }
  const turn = d.turn || {};
  const out = [];
  turns.forEach((t, i) => {
    const last = i === turns.length - 1;
    if (t.kind === 'arrived') out.push(`<li class="sys">Arrived in the inbox at ${hhmm(t.at)}</li>`);
    if (t.kind === 'you') out.push(`<li class="msg you"><div class="by">You · ${hhmm(t.at)}</div><p>${esc(t.text)}</p></li>`);
    if (t.steps) out.push(`<li class="work">Worked for ${took(t.busy)} · ${t.steps} ${t.steps === 1 ? 'step' : 'steps'}${t.waited && !(last && d.state === 'waiting') ? ' · waited for your decision' : ''}</li>`);
    if (t.reply && !t.tail) out.push(`<li class="msg agent"><div class="by">AP Desk · ${hhmm(t.replyAt)}</div><p>${esc(t.reply)}</p></li>`);
    if (last && t.tail && turn.state === 'running') out.push('<li class="work">Working…</li>');
    if (last && turn.state === 'waiting' && d.state === 'waiting') out.push('<li class="work">Waiting for your decision above</li>');
    if (last && turn.state === 'failed') out.push(`<li class="err">Claude could not answer: ${esc(turn.error || 'the turn failed without a reason')}</li>`);
  });
  if (!out.length) out.push(`<li class="sys">${d.started ? 'AP Desk has not written anything about ' + esc(d.doc) + ' yet.' : 'Nothing about ' + esc(d.doc) + ' yet: AP Desk writes here once it reads the invoice.'}</li>`);
  const html = out.join('');
  if (html !== threadHtml) { threadHtml = html; $('thread').innerHTML = html; }
  $('talk-h').textContent = 'Conversation about ' + d.doc;
}
function renderAsk(d) {
  const live = S.mode === 'live', ta = $('ask-text'), send = $('ask-send'), note = $('ask-note');
  ta.placeholder = `Ask about ${d.doc}…`; $('ask-label').textContent = `Ask AP Desk about ${d.doc}`;
  const turn = d.turn || {};
  let text = '', err = false, html = false, disabled = false;
  const askedRunning = turn.state === 'running' && turn.asked;
  const justSent = answering && Date.now() - answering < 5000;
  if (askedRunning) askNote = null;
  if (!justSent) answering = 0;
  if (!live) { disabled = true; html = true; text = 'Questions need a live run: set ANTHROPIC_API_KEY and start with <code>serve --live</code>.'; }
  else if (!d.started) { disabled = true; text = `AP Desk has not read ${d.doc} yet; ask once it has.`; }
  else if (askedRunning || (justSent && turn.state !== 'failed')) text = 'AP Desk is answering…';
  else if (turn.state === 'failed' && turn.asked) { text = 'Claude could not answer: ' + (turn.error || 'the turn failed without a reason'); err = true; }
  else if (askNote) { text = askNote.text; err = askNote.err; }
  ta.disabled = disabled; send.disabled = disabled;
  if (html) note.innerHTML = text; else note.textContent = text;
  note.className = 'note' + (err ? ' err' : '');
}
$('ask').onsubmit = async e => {
  e.preventDefault();
  const text = $('ask-text').value.trim(), doc = chosen;
  if (!text || !doc) return;
  $('ask-send').disabled = true; askNote = null;
  try {
    const r = await fetch('/api/surface/commands', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ surface: 'web', origin: { kind: 'pointer' }, command: { type: 'send', sessionId: doc, text } }) });
    const body = await r.json().catch(() => ({}));
    if (!r.ok) askNote = { text: 'Your question was not sent: ' + (body.detail || body.title || 'HTTP ' + r.status), err: true };
    else { $('ask-text').value = ''; if (body.type === 'queued') askNote = { text: body.text, err: false }; else answering = Date.now(); }
  } catch (err) { askNote = { text: 'Your question was not sent: ' + err.message, err: true }; }
  $('ask-send').disabled = false;
  refreshDoc();
};
$('ask-text').onkeydown = e => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); $('ask').requestSubmit(); } };
async function refreshDoc() {
  const doc = chosen; if (!doc || !S) return;
  let d;
  try { d = await getJson('/ap/doc/' + doc); } catch (e) { return; }
  if (chosen !== doc) return;
  D = d; renderFound(d);
  if (d.exception && d.state === 'waiting') {
    try {
      const r = await getJson(`/api/surface/decisions?surface=web&session=${encodeURIComponent(doc)}`);
      if (chosen !== doc) return;
      card = (r.decisions || []).find(c => c.call && c.call.tool === 'raise_exception') || null;
    } catch (e) { card = null; }
  }
  renderDecision(d);
  let entries = [];
  if (d.started) { try { entries = (await getJson(`/api/surface/sessions/${encodeURIComponent(doc)}/thread?surface=web`)).entries; } catch (e) { entries = []; } }
  if (chosen !== doc) return;
  renderThread(d, entries); renderAsk(d);
}
async function tick() {
  try { S = await getJson('/ap/state'); $('offline').hidden = true; }
  catch (e) { $('offline').hidden = false; $('offline').textContent = 'AP Desk is not answering. Is the app still running?'; return false; }
  renderHeader(); renderList(); renderEmpty();
  if (chosen) await refreshDoc();
  return true;
}
$('start').onclick = async () => {
  $('start').disabled = true;
  try { await fetch('/ap/start', { method: 'POST' }); } catch (e) {}
  tick();
};
// While the app is down, ask less often.
async function loop() { setTimeout(loop, (await tick()) ? 1200 : 5000); }
loop();
</script>
</body>
</html>
"##;

#[cfg(test)]
mod tests {
    use super::{evidence_line, money, plain_error};
    use serde_json::json;

    #[test]
    fn amounts_read_as_a_clerk_reads_them() {
        assert_eq!(
            [
                money(1171.68),
                money(99.95),
                money(1000.0),
                money(-12.5),
                money(1_234_567.891)
            ],
            ["1,171.68", "99.95", "1,000.00", "-12.50", "1,234,567.89"].map(String::from)
        );
    }

    #[test]
    fn a_failed_turn_says_what_went_wrong_in_words() {
        assert_eq!(
            [
                plain_error(
                    "model.unauthorized: Anthropic API error (401): invalid x-api-key [type=authentication_error]"
                ),
                plain_error("the daily spending cap is reached"),
            ],
            [
                "the API key was refused; check ANTHROPIC_API_KEY (Anthropic API error (401): invalid x-api-key)",
                "the daily spending cap is reached",
            ]
            .map(String::from)
        );
    }

    #[test]
    fn the_evidence_is_the_line_the_exception_is_about() {
        let read = json!({
            "content": "<untrusted-content>\n[1] White and Sons\n[2] SUB_TOTAL: 1161.65 $\n[3] TOTAL:\n[4] 1,171.68 $\n</untrusted-content>",
            "blocks": [
                { "line": 1, "box": [15.0, 22.0, 110.0, 45.0] },
                { "line": 2, "box": [300.0, 700.0, 520.0, 720.0] },
                { "line": 3, "box": [300.0, 730.0, 360.0, 750.0] },
                { "line": 4, "box": [420.0, 731.0, 520.0, 749.0] },
            ],
        });
        let fields = json!({ "supplier": "White and Sons", "total": 1171.68 });
        assert_eq!(
            [
                evidence_line("over_limit", &fields, Some(&read)),
                evidence_line("unknown_supplier", &fields, Some(&read)),
                evidence_line("over_limit", &json!({ "total": 5.0 }), Some(&read)),
            ],
            [
                Some(
                    json!({ "box": [300.0, 730.0, 520.0, 750.0], "label": "The total as printed" })
                ),
                Some(
                    json!({ "box": [15.0, 22.0, 110.0, 45.0], "label": "The supplier as printed" })
                ),
                None,
            ]
        );
    }
}
