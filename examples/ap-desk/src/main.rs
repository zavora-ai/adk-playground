//! AP Desk: an accounts payable agent that reads supplier invoices on this computer, matches
//! them against the ERP, posts the clean ones and puts the rest to the clerk.

mod data;
mod desk;
mod eval;
mod serve;
mod tape;
mod terminal;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::data::Books;
use crate::tape::{Mode, Tape};
use crate::terminal::CardMode;

#[derive(Parser)]
#[command(
    version,
    about = "An accounts payable agent over a day's supplier invoices"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Args, Clone)]
pub struct Common {
    /// Run live against Claude and record its answers; without it, the recorded answers are
    /// replayed and no key is needed
    #[arg(long)]
    pub live: bool,
    /// Work only the first N documents of the inbox
    #[arg(long)]
    pub docs: Option<usize>,
    /// The agent
    #[arg(long, default_value = "claude-sonnet-5-5")]
    pub agent_model: String,
    /// The model `vision_describe` sends a crop to
    #[arg(long, default_value = "claude-sonnet-5-5")]
    pub crop_model: String,
    /// The kernel's daily spending cap on model calls, in US dollars
    #[arg(long, default_value_t = 5.0)]
    pub daily_usd: f64,
    /// Where the journal, the filed exceptions and the results go
    #[arg(long, default_value = "out")]
    pub out: PathBuf,
}

#[derive(Subcommand)]
enum Command {
    /// Downloads FATURA (363 MB, checked by size and SHA-256) and builds the inbox, the ERP
    /// and the expected answers from it
    Fetch,
    /// Works the inbox headless and prints the measured results
    Run {
        #[command(flatten)]
        common: Common,
        /// What happens to each exception held for the clerk
        #[arg(long, value_enum, default_value_t = CardMode::Hold)]
        cards: CardMode,
    },
    /// The web home and the AP Desk page while the inbox is worked
    Serve {
        #[command(flatten)]
        common: Common,
        #[arg(long, default_value_t = 8817)]
        port: u16,
    },
}

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    adk_core::ensure_crypto_provider();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,adk_kernel=error")),
        )
        .with_writer(std::io::stderr)
        .init();
    match Cli::parse().command {
        Command::Fetch => {
            println!("fetching FATURA (Zenodo record 8261508) and building the inbox");
            data::fetch(&root()).await
        }
        Command::Run { common, cards } => run(common, cards).await,
        Command::Serve { common, port } => serve::serve(common, port).await,
    }
}

/// What a run and the web home share.
pub struct App {
    pub tape: Arc<Tape>,
    pub books: Books,
    pub desk: Arc<desk::Desk>,
    pub started: Arc<Mutex<Vec<String>>>,
}

impl App {
    pub async fn build(common: &Common) -> anyhow::Result<Self> {
        let root = root();
        let books = Books::load(&root)?;
        let mode = if common.live {
            Mode::Live
        } else {
            Mode::Replay
        };
        if mode == Mode::Live && std::env::var("ANTHROPIC_API_KEY").is_err() {
            anyhow::bail!(
                "--live needs ANTHROPIC_API_KEY (see .env.example); without --live the recorded answers replay"
            );
        }
        let tape = Tape::open(&root.join("recordings/tape.json"), mode)?;
        let settings = desk::Settings {
            dir: common.out.clone(),
            agent_model: common.agent_model.clone(),
            crop_model: common.crop_model.clone(),
            daily_usd: common.daily_usd,
        };
        let desk = Arc::new(desk::Desk::build(&settings, &tape, books.erp.clone()).await?);
        Ok(Self {
            tape,
            books,
            desk,
            started: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Works the inbox in arrival order, one invoice at a time, as a clerk's queue is worked.
    pub async fn work(&self, limit: Option<usize>) -> anyhow::Result<()> {
        let inbox = data::inbox_dir(&root());
        for expected in self.books.expected.iter().take(limit.unwrap_or(usize::MAX)) {
            let page = std::fs::read(inbox.join(format!("{}.jpg", expected.doc)))?;
            self.started
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(expected.doc.clone());
            self.desk.start(&expected.doc, page).await?;
            let deadline = Instant::now() + Duration::from_secs(300);
            while !self.desk.settled(&expected.doc).await && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
        Ok(())
    }
}

async fn run(common: Common, cards: CardMode) -> anyhow::Result<()> {
    let started_at = Instant::now();
    let app = App::build(&common).await?;
    let count = common
        .docs
        .unwrap_or(app.books.expected.len())
        .min(app.books.expected.len());
    println!(
        "working {count} documents of the inbox ({})",
        if app.tape.mode == Mode::Live {
            "live: Claude's answers are recorded"
        } else {
            "replaying recorded answers"
        }
    );
    app.desk.kernel.start().await?;
    let cancel = CancellationToken::new();
    tokio::spawn(terminal::narrate(
        app.desk.sessions.clone(),
        Arc::clone(&app.started),
        cancel.clone(),
    ));
    tokio::spawn(terminal::cards(
        app.desk.approvals.clone(),
        cards,
        cancel.clone(),
    ));
    app.work(common.docs).await?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    cancel.cancel();
    app.desk.kernel.stop().await?;
    app.tape.save()?;
    report(&common, &app, started_at.elapsed()).await
}

async fn report(common: &Common, app: &App, took: Duration) -> anyhow::Result<()> {
    let docs: Vec<String> = app
        .started
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let mut outcomes = Vec::new();
    for doc in &docs {
        outcomes.push(eval::Outcome::from_calls(doc, &app.desk.calls(doc).await));
    }
    let report = eval::score(&app.books, &outcomes);
    let spend = app.tape.spend();
    println!();
    println!("results ({} documents in {:.0?})", report.documents, took);
    for field in eval::FIELDS {
        let (ok, all) = report.fields.get(field).copied().unwrap_or_default();
        println!("  {field:<10} {ok:>3} of {all} right");
    }
    for (arrival, (ok, all)) in &report.whole_by_arrival {
        println!("  every field right, {arrival:<13} {ok} of {all}");
    }
    println!(
        "  decisions right                {} of {}",
        report.decisions_right, report.documents
    );
    println!(
        "  exceptions caught              {} of {} the policy calls for; {} clean invoices held, {} posted that should not be",
        report.exceptions_caught,
        report.exceptions_expected,
        report.false_exceptions,
        report.wrongly_posted
    );
    for (want, got) in &report.confusion {
        let row: Vec<String> = got.iter().map(|(g, n)| format!("{g} {n}")).collect();
        println!("    expected {want:<17} → {}", row.join(", "));
    }
    println!("  crops sent to Claude           {}", report.crops);
    for (role, s) in &spend {
        println!(
            "  {role:<6} {:<18} {:>4} calls {:>9} in {:>7} out  ${:.4}",
            s.model, s.calls, s.input_tokens, s.output_tokens, s.usd
        );
    }
    println!(
        "  total model spend              ${:.4}{}",
        spend.values().map(|s| s.usd).sum::<f64>(),
        if app.tape.mode == Mode::Replay {
            " (the recorded live run's; this replay spent nothing)"
        } else {
            ""
        }
    );
    std::fs::create_dir_all(&common.out)?;
    let results = json!({
        "mode": if app.tape.mode == Mode::Live { "live" } else { "replay" },
        "expected": app.books.expected.iter().take(docs.len()).collect::<Vec<_>>(),
        "outcomes": outcomes,
        "report": report,
        "spend": spend,
    });
    let path = common.out.join("results.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&results)?)?;
    println!("  results                        {}", path.display());
    println!(
        "  journal                        {}",
        common.out.join("journal.md").display()
    );
    Ok(())
}
