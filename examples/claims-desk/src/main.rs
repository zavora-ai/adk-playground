//! Claims Desk: a motor claims agent that assesses damage from the claim's photos, checks
//! the photos belong to the claim, and drafts the estimate for the adjuster.

mod data;
mod desk;
mod serve;
mod tape;
mod terminal;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::data::{Claim, Outcome, Severity};
use crate::tape::{Mode, Tape};
use crate::terminal::CardMode;

#[derive(Parser)]
#[command(version, about = "A motor claims agent over a day's claims")]
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
    /// Work only the first N claims
    #[arg(long)]
    pub claims: Option<usize>,
    /// The agent
    #[arg(long, default_value = "claude-sonnet-5-5")]
    pub agent_model: String,
    /// The model `vision_describe` asks to assess a photo
    #[arg(long, default_value = "claude-sonnet-5-5")]
    pub assessor_model: String,
    #[arg(long, default_value_t = 5.0)]
    pub daily_usd: f64,
    #[arg(long, default_value = "out")]
    pub out: PathBuf,
}

#[derive(Subcommand)]
enum Command {
    /// Downloads the twenty Commons photos (each checked by size and SHA-1) and writes the
    /// claims' photos
    Fetch,
    /// Works the day's claims headless and prints the measured results
    Run {
        #[command(flatten)]
        common: Common,
        /// What happens to each estimate and referral held for the adjuster
        #[arg(long, value_enum, default_value_t = CardMode::Hold)]
        cards: CardMode,
    },
    /// The web home and the Claims Desk page while the claims are worked
    Serve {
        #[command(flatten)]
        common: Common,
        #[arg(long, default_value_t = 8819)]
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
            println!("fetching the claim photos from Wikimedia Commons");
            data::fetch(&root()).await
        }
        Command::Run { common, cards } => run(common, cards).await,
        Command::Serve { common, port } => serve::serve(common, port).await,
    }
}

pub struct App {
    pub tape: Arc<Tape>,
    pub claims: Vec<Claim>,
    pub desk: Arc<desk::Desk>,
    pub started: Arc<Mutex<Vec<String>>>,
}

impl App {
    pub async fn build(common: &Common) -> anyhow::Result<Self> {
        let root = root();
        let mut claims = data::claims();
        claims.truncate(common.claims.unwrap_or(claims.len()));
        if !data::photo_path(&root, &claims[0].photos[0].name).exists() {
            anyhow::bail!("the claim photos are missing; run `cargo run --release -- fetch` first");
        }
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
            assessor_model: common.assessor_model.clone(),
            daily_usd: common.daily_usd,
        };
        let desk = Arc::new(desk::Desk::build(&settings, &tape, &claims).await?);
        Ok(Self {
            tape,
            claims,
            desk,
            started: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Works the claims in the order they arrive, one at a time, so an earlier claim's photos
    /// are in the visual memory when a later claim's are checked.
    pub async fn work(&self) -> anyhow::Result<()> {
        let root = root();
        for claim in &self.claims {
            let photos = claim
                .photos
                .iter()
                .map(|p| std::fs::read(data::photo_path(&root, &p.name)))
                .collect::<Result<Vec<_>, _>>()?;
            self.started
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(claim.id.clone());
            self.desk.start(claim, photos).await?;
            let deadline = Instant::now() + Duration::from_secs(400);
            while !self.desk.settled(&claim.id).await && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        }
        Ok(())
    }
}

/// What the agent made of a claim: its estimate or referral.
pub fn outcome(calls: &[(String, Value)]) -> (Option<String>, Option<Value>) {
    let mut last = (None, None);
    for (name, args) in calls {
        match name.as_str() {
            "draft_estimate" => last = (Some("estimate".to_string()), Some(args.clone())),
            "refer_claim" => {
                last = (
                    args["reason"].as_str().map(str::to_string),
                    Some(args.clone()),
                )
            }
            _ => {}
        }
    }
    last
}

fn severity_of(text: &str) -> Option<Severity> {
    match text {
        "minor" => Some(Severity::Minor),
        "moderate" => Some(Severity::Moderate),
        "major" => Some(Severity::Major),
        _ => None,
    }
}

async fn run(common: Common, cards: CardMode) -> anyhow::Result<()> {
    let started = Instant::now();
    let app = App::build(&common).await?;
    println!(
        "working {} claims ({})",
        app.claims.len(),
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
    app.work().await?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    cancel.cancel();
    app.desk.kernel.stop().await?;
    app.tape.save()?;

    let mut rows = Vec::new();
    let (mut right, mut sev_exact, mut sev_near, mut sev_n) = (0, 0, 0, 0);
    let (mut side_ok, mut side_n) = (0, 0);
    let mut confusion =
        std::collections::BTreeMap::<String, std::collections::BTreeMap<String, usize>>::new();
    for claim in &app.claims {
        let (got, args) = outcome(&app.desk.calls(&claim.id).await);
        let want = match claim.outcome {
            Outcome::Estimate => "estimate",
            Outcome::ReusedPhoto => "reused_photo",
            Outcome::DifferentVehicle => "different_vehicle",
        };
        let got_text = got.clone().unwrap_or_else(|| "none".into());
        *confusion
            .entry(want.into())
            .or_default()
            .entry(got_text.clone())
            .or_default() += 1;
        if got_text == want {
            right += 1;
        }
        let args = args.unwrap_or(Value::Null);
        if want == "estimate" && got_text == "estimate" {
            if let Some(s) = args["severity"].as_str().and_then(severity_of) {
                sev_n += 1;
                sev_exact += usize::from(s == claim.severity);
                sev_near += usize::from((s as i32 - claim.severity as i32).abs() <= 1);
            }
            if let Some(side) = &claim.side {
                side_n += 1;
                let sides: Vec<String> = args["sides"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_lowercase))
                    .collect();
                let hit = match side.as_str() {
                    "side" => sides
                        .iter()
                        .any(|s| s.contains("left") || s.contains("right")),
                    other => sides.iter().any(|s| s.contains(other)),
                };
                side_ok += usize::from(hit);
            }
        }
        rows.push(json!({ "claim": claim, "outcome": got, "args": args }));
    }
    let spend = app.tape.spend();
    println!();
    println!(
        "results ({} claims in {:.0?})",
        app.claims.len(),
        started.elapsed()
    );
    println!(
        "  outcomes right                    {right} of {}",
        app.claims.len()
    );
    for (want, got) in &confusion {
        let row: Vec<String> = got.iter().map(|(g, n)| format!("{g} {n}")).collect();
        println!("    expected {want:<18} → {}", row.join(", "));
    }
    println!(
        "  severity, against Commons         {sev_exact} of {sev_n} the same; {sev_near} of {sev_n} within one step"
    );
    println!("  damaged side, where Commons names it  {side_ok} of {side_n}");
    for (role, s) in &spend {
        println!(
            "  {role:<9} {:<18} {:>4} calls {:>8} in {:>6} out  ${:.4}",
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
    let path = common.out.join("results.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "mode": if app.tape.mode == Mode::Live { "live" } else { "replay" },
            "claims": rows,
            "outcomesRight": right,
            "severity": { "exact": sev_exact, "withinOne": sev_near, "of": sev_n },
            "side": { "right": side_ok, "of": side_n },
            "spend": spend,
        }))?,
    )?;
    println!("  results                           {}", path.display());
    Ok(())
}
