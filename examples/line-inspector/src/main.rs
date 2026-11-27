//! Line Inspector: every tea light on a replayed conveyor scored on this computer against
//! known-good candles, rejects segmented and measured, borderline parts given a second look by
//! two models, and disagreements put to the quality engineer.

mod agent;
mod data;
mod inspect;
mod line;
mod serve;
mod tape;
mod terminal;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adk_vision::onnx::{OnnxOptions, Sam2, SigLip2};
use adk_vision::{FetchConfig, InMemoryAcceptanceStore, ManifestSet, ModelFetcher, VisionRuntime};
use clap::{Parser, Subcommand};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::line::{Book, Line, TrayFeed};
use crate::tape::{Mode, Tape};
use crate::terminal::CardMode;

#[derive(Parser)]
#[command(version, about = "A quality agent over a candle line")]
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
    /// Inspect only the first N trays of the line
    #[arg(long)]
    pub trays: Option<usize>,
    /// The agent
    #[arg(long, default_value = "claude-sonnet-5-5")]
    pub agent_model: String,
    /// The reasoner `vision_describe` asks first
    #[arg(long, default_value = "claude-sonnet-5-5")]
    pub reasoner_model: String,
    /// The second opinion
    #[arg(long, default_value = "claude-haiku-5-5")]
    pub second_model: String,
    #[arg(long, default_value_t = 5.0)]
    pub daily_usd: f64,
    #[arg(long, default_value = "out")]
    pub out: PathBuf,
}

#[derive(Subcommand)]
enum Command {
    /// Downloads VisA's candle class (the first 129 MB of the archive, checked by SHA-256)
    Fetch,
    /// Runs the line headless and prints the measured results
    Run {
        #[command(flatten)]
        common: Common,
        /// What happens to a part put to the engineer
        #[arg(long, value_enum, default_value_t = CardMode::Hold)]
        cards: CardMode,
    },
    /// The line page, where the engineer starts the line, decides parts and asks about them;
    /// the trays come through at one a second once started
    Serve {
        #[command(flatten)]
        common: Common,
        #[arg(long, default_value_t = 8818)]
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
            println!("fetching VisA's candle class");
            data::fetch(&root()).await
        }
        Command::Run { common, cards } => run(common, cards).await,
        Command::Serve { common, port } => serve::serve(common, port).await,
    }
}

fn fetch(fetcher: &ModelFetcher, id: &str) -> anyhow::Result<adk_vision::VerifiedModel> {
    let manifest = ManifestSet::builtin()?
        .get(id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("{id} is not built in"))?;
    Ok(fetcher.fetch_blocking(&manifest)?)
}

/// Borderline parts as the line finds them, with their tray's image.
type Borderline = tokio::sync::mpsc::UnboundedReceiver<(line::Part, Vec<u8>)>;

pub struct App {
    pub tape: Arc<Tape>,
    pub trays: Vec<data::Tray>,
    pub book: Arc<Book>,
    pub desk: Arc<agent::Desk>,
    pub inspector: Arc<VisionRuntime>,
    pub line: Arc<Line>,
    pub thresholds: inspect::Thresholds,
    incoming: tokio::sync::Mutex<Option<Borderline>>,
}

impl App {
    pub async fn build(common: &Common) -> anyhow::Result<Self> {
        let root = root();
        let mut trays = data::line(&root)?;
        trays.truncate(common.trays.unwrap_or(trays.len()));
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
        println!(
            "loading SigLIP 2 and SAM 2.1 Tiny (fetched by revision and checked on first run)…"
        );
        let (siglip, sam) = tokio::task::spawn_blocking(|| -> anyhow::Result<_> {
            let fetcher = ModelFetcher::new(
                FetchConfig::from_env(),
                Arc::new(InMemoryAcceptanceStore::new()),
            );
            let siglip = SigLip2::new(
                &fetch(&fetcher, "siglip2-base-patch16-256-quantized")?,
                &OnnxOptions::default(),
            )?;
            let sam = Sam2::new(
                &fetch(&fetcher, "sam2.1-hiera-tiny-q4")?,
                &OnnxOptions::default(),
            )?;
            Ok((siglip, sam))
        })
        .await??;
        let embedder = Arc::new(
            VisionRuntime::builder()
                .embedder(Arc::new(siglip))
                .build()?,
        );
        let bank: Vec<PathBuf> = data::BANK.map(|n| data::normal_image(&root, n)).collect();
        let calibration: Vec<PathBuf> = data::CALIBRATION
            .map(|n| data::normal_image(&root, n))
            .collect();
        let scorer = Arc::new(
            inspect::Scorer::open(embedder, &data::data_dir(&root), &bank, &calibration).await?,
        );
        let thresholds = scorer.thresholds;
        let inspector = Arc::new(
            VisionRuntime::builder()
                .detector(Arc::new(inspect::Inspector::new(scorer)))
                .build()?,
        );
        let sam = Arc::new(sam);
        let segmenter = Arc::new(
            VisionRuntime::builder()
                .segmenter(Arc::clone(&sam) as Arc<dyn adk_vision::Segmenter>)
                .build()?,
        );
        let (book, incoming) = Book::new();
        let settings = agent::Settings {
            dir: common.out.clone(),
            agent_model: common.agent_model.clone(),
            reasoner_model: common.reasoner_model.clone(),
            second_model: common.second_model.clone(),
            daily_usd: common.daily_usd,
        };
        let desk = Arc::new(agent::Desk::build(&settings, &tape, Arc::clone(&book), sam).await?);
        let line = Arc::new(Line {
            trays: trays.clone(),
            book: Arc::clone(&book),
            segmenter,
        });
        Ok(Self {
            tape,
            trays,
            book,
            desk,
            inspector,
            line,
            thresholds,
            incoming: tokio::sync::Mutex::new(Some(incoming)),
        })
    }

    /// Starts the agent on each borderline part as the line finds it.
    pub async fn dispatch(&self) {
        let Some(mut incoming) = self.incoming.lock().await.take() else {
            return;
        };
        let desk = Arc::clone(&self.desk);
        tokio::spawn(async move {
            while let Some((part, bytes)) = incoming.recv().await {
                if let Err(err) = desk.start(&part, bytes).await {
                    eprintln!("[{}] the second look could not start: {err}", part.id);
                }
            }
        });
    }

    pub async fn settle(&self, limit: Duration) {
        let deadline = Instant::now() + limit;
        tokio::time::sleep(Duration::from_secs(1)).await;
        while Instant::now() < deadline {
            let mut open = 0;
            for id in self.desk.started() {
                if !self.desk.settled(&id).await {
                    open += 1;
                }
            }
            if open == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    }
}

async fn run(common: Common, cards: CardMode) -> anyhow::Result<()> {
    let started = Instant::now();
    let app = App::build(&common).await?;
    println!(
        "running {} trays down line 2: pass below {:.3}, reject above {:.3} (0.45 and 0.75 on the board's scale); {}",
        app.trays.len(),
        app.thresholds.pass,
        app.thresholds.reject,
        if app.tape.mode == Mode::Live {
            "live: Claude's answers are recorded"
        } else {
            "replaying recorded answers"
        }
    );
    app.desk.kernel.start().await?;
    let cancel = CancellationToken::new();
    tokio::spawn(terminal::narrate(app.desk.clone(), cancel.clone()));
    tokio::spawn(terminal::cards(
        app.desk.approvals.clone(),
        cards,
        cancel.clone(),
    ));
    app.dispatch().await;
    let summary = line::run(
        TrayFeed::new(app.trays.clone(), false),
        Arc::clone(&app.inspector),
        Arc::clone(&app.line),
    )
    .await?;
    println!("[line] {} trays inspected", summary.observations);
    app.settle(Duration::from_secs(600)).await;
    cancel.cancel();
    app.desk.kernel.stop().await?;
    app.tape.save()?;
    report(&common, &app, started.elapsed()).await
}

/// A part's final call: the line's, the agent's, or `engineer` while one is waiting.
pub async fn verdict(app: &App, part: &line::Part) -> String {
    if part.stage != "borderline" {
        return part.stage.clone();
    }
    if let Some(v) = app
        .desk
        .verdicts
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&part.id)
    {
        return v["verdict"].as_str().unwrap_or("pass").to_string();
    }
    let calls = app.desk.calls(&part.id).await;
    if calls.iter().any(|(name, _)| name == "ask_engineer") {
        "engineer".into()
    } else {
        "undecided".into()
    }
}

async fn report(common: &Common, app: &App, took: Duration) -> anyhow::Result<()> {
    let parts = app.book.all();
    let mut rows = Vec::new();
    let (mut defective, mut good) = (0usize, 0usize);
    let (mut caught, mut false_rejects) = (0usize, 0usize);
    let mut stage = std::collections::BTreeMap::<(bool, String), usize>::new();
    let mut second = std::collections::BTreeMap::<String, usize>::new();
    for part in &parts {
        let v = verdict(app, part).await;
        *stage
            .entry((part.defective, part.stage.clone()))
            .or_default() += 1;
        if part.stage == "borderline" {
            let right = match v.as_str() {
                "reject" => part.defective,
                "pass" => !part.defective,
                _ => false,
            };
            *second
                .entry(format!(
                    "{v}{}",
                    if v == "pass" || v == "reject" {
                        if right { " (right)" } else { " (wrong)" }
                    } else {
                        ""
                    }
                ))
                .or_default() += 1;
        }
        let stopped = v != "pass" && v != "undecided";
        if part.defective {
            defective += 1;
            caught += usize::from(stopped);
        } else {
            good += 1;
            false_rejects += usize::from(stopped);
        }
        rows.push(json!({ "part": part, "verdict": v }));
    }
    let ious: Vec<f32> = parts.iter().filter_map(|p| p.iou).collect();
    let sizes: Vec<f64> = parts
        .iter()
        .filter(|p| p.defective)
        .filter_map(|p| p.longest_mm)
        .collect();
    let mean = |v: &[f32]| {
        if v.is_empty() {
            0.0
        } else {
            v.iter().sum::<f32>() / v.len() as f32
        }
    };
    let spend = app.tape.spend();
    println!();
    println!(
        "results ({} trays, {} candles, in {:.0?})",
        app.trays.len(),
        parts.len(),
        took
    );
    println!("  defective candles (VisA's masks)  {defective}; good {good}");
    for ((d, s), n) in &stage {
        println!(
            "    on this computer: {} {s:<10} {n}",
            if *d { "defective" } else { "good     " }
        );
    }
    for (k, n) in &second {
        println!("    second look: {k:<16} {n}");
    }
    println!(
        "  defects stopped (rejected or with the engineer)  {caught} of {defective} (recall {:.2})",
        caught as f64 / defective.max(1) as f64
    );
    println!(
        "  good candles stopped                             {false_rejects} of {good} (false-reject rate {:.3})",
        false_rejects as f64 / good.max(1) as f64
    );
    println!(
        "  defect masks (SAM 2.1 vs VisA)    mean IoU {:.2} over {} rejects",
        mean(&ious),
        ious.len()
    );
    if !sizes.is_empty() {
        println!(
            "  defect sizes                      {:.1}–{:.1} mm across",
            sizes.iter().copied().fold(f64::MAX, f64::min),
            sizes.iter().copied().fold(0.0, f64::max)
        );
    }
    for (role, s) in &spend {
        println!(
            "  {role:<8} {:<18} {:>4} calls {:>8} in {:>6} out  ${:.4}",
            s.model, s.calls, s.input_tokens, s.output_tokens, s.usd
        );
    }
    println!(
        "  total model spend                 ${:.4}{}",
        spend.values().map(|s| s.usd).sum::<f64>(),
        if app.tape.mode == Mode::Replay {
            " (the recorded live run's; this replay spent nothing)"
        } else {
            ""
        }
    );
    std::fs::create_dir_all(&common.out)?;
    let log: Vec<String> = app
        .book
        .log
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    std::fs::write(
        common.out.join("reject-log.md"),
        format!(
            "# Line 2 reject log\n\n{}\n",
            log.iter()
                .map(|l| format!("- {l}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    )?;
    let results = json!({
        "mode": if app.tape.mode == Mode::Live { "live" } else { "replay" },
        "thresholds": app.thresholds,
        "parts": rows,
        "recall": caught as f64 / defective.max(1) as f64,
        "falseRejectRate": false_rejects as f64 / good.max(1) as f64,
        "meanIou": mean(&ious),
        "spend": spend,
    });
    let path = common.out.join("results.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&results)?)?;
    println!("  results                           {}", path.display());
    Ok(())
}
