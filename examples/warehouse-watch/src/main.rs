//! Warehouse Watch: a shift supervisor's safety agent over four cameras of NVIDIA's
//! PhysicalAI-SmartSpaces warehouse.

mod agent;
mod data;
mod eval;
mod serve;
mod tape;
mod terminal;
mod watch;

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adk_artifact::InMemoryArtifactService;
use adk_core::egress::EgressPolicy;
use adk_vision::onnx::{OnnxOptions, RfDetr};
use adk_vision::{
    EgressConfig, EgressGate, FetchConfig, InMemoryAcceptanceStore, InMemoryEgressRecorder,
    LlmDescriber, ManifestSet, ModelFetcher, VisionRuntime,
};
use clap::{Parser, Subcommand};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::tape::{Mode, Tape};
use crate::terminal::CardMode;
use crate::watch::{Camera, Incidents, clock};

#[derive(Parser)]
#[command(
    version,
    about = "A shift supervisor's safety agent over four warehouse cameras"
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
    /// Watch only the first N seconds of the five-minute shift
    #[arg(long)]
    pub seconds: Option<u64>,
    /// The agent that reviews each incident
    #[arg(long, default_value = "claude-sonnet-5-5")]
    pub agent_model: String,
    /// The model that spots forklifts and pallet trucks
    #[arg(long, default_value = "claude-haiku-5-5")]
    pub spot_model: String,
    /// The model `vision_video` asks about a clip
    #[arg(long, default_value = "claude-sonnet-5-5")]
    pub video_model: String,
    /// The kernel's daily spending cap on model calls, in US dollars
    #[arg(long, default_value_t = 5.0)]
    pub daily_usd: f64,
    /// Where the shift log, incident reports and results go
    #[arg(long, default_value = "out")]
    pub out: PathBuf,
}

#[derive(Subcommand)]
enum Command {
    /// Downloads the four cameras, their calibration and the ground truth (1.27 GB) at the
    /// pinned revision, each file checked by size and SHA-256
    Fetch,
    /// Watches the shift headless and prints the measured results
    Run {
        #[command(flatten)]
        common: Common,
        /// What happens to each incident held for the supervisor
        #[arg(long, value_enum, default_value_t = CardMode::Hold)]
        cards: CardMode,
    },
    /// The Warehouse Watch page: the four cameras, each close approach with the agent's review,
    /// the supervisor's decisions and questions; the shift starts from the page
    Serve {
        #[command(flatten)]
        common: Common,
        #[arg(long, default_value_t = 8816)]
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
            println!(
                "fetching {}@{} {}",
                data::REPO,
                &data::REVISION[..10],
                data::SCENE
            );
            data::fetch(&root()).await
        }
        Command::Run { common, cards } => run(common, cards).await,
        Command::Serve { common, port } => serve::serve(common, port).await,
    }
}

/// Everything a run or the web home shares.
pub struct App {
    pub tape: Arc<Tape>,
    pub detector: Arc<VisionRuntime>,
    pub cameras: Vec<Arc<Camera>>,
    pub incidents: Arc<Incidents>,
    pub desk: agent::Desk,
    pub incoming: tokio::sync::mpsc::UnboundedReceiver<watch::Incident>,
}

impl App {
    async fn build(common: &Common) -> anyhow::Result<Self> {
        let root = root();
        data::check(&root)?;
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
        let floors = data::floors(&root)?;
        println!("loading RF-DETR Nano (fetched by revision and checked on first run)…");
        let manifest = ManifestSet::builtin()?
            .get("rf-detr-n")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("rf-detr-n is not in the built-in manifest"))?;
        let model = tokio::task::spawn_blocking(move || {
            ModelFetcher::new(
                FetchConfig::from_env(),
                Arc::new(InMemoryAcceptanceStore::new()),
            )
            .fetch_blocking(&manifest)
        })
        .await??;
        let rf_detr = Arc::new(RfDetr::new(&model, &OnnxOptions::default())?.with_threshold(0.45));
        let detector = Arc::new(VisionRuntime::builder().detector(rf_detr).build()?);

        // The spotter's images go through the egress gate like any other: with no rules they
        // leave as they are, and each release is recorded.
        let spot_gate = EgressGate::new(
            EgressPolicy::allow_private_networks(),
            EgressConfig::default(),
            Arc::new(InMemoryEgressRecorder::new()),
        );
        let spot_runtime = Arc::new(
            VisionRuntime::builder()
                .remote_describer(Arc::new(
                    LlmDescriber::remote(agent::claude(&common.spot_model, &tape, "spot")?)?
                        .with_max_output_tokens(1_500),
                ))
                .egress_gate(spot_gate)
                .build()?,
        );
        let spotter: Arc<dyn watch::Spotter> = Arc::new(watch::ClaudeSpotter {
            runtime: spot_runtime,
            tape: Arc::clone(&tape),
        });

        let artifacts = Arc::new(InMemoryArtifactService::new());
        let (incidents, incoming) = Incidents::new(artifacts.clone());
        let settings = agent::Settings {
            dir: common.out.clone(),
            agent_model: common.agent_model.clone(),
            video_model: common.video_model.clone(),
            daily_usd: common.daily_usd,
        };
        let desk = agent::Desk::build(&settings, &tape, artifacts, Arc::clone(&incidents)).await?;
        let cameras = data::CAMERAS
            .iter()
            .map(|name| {
                let floor = floors
                    .get(*name)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("no calibration for {name}"))?;
                Ok(Camera::new(
                    name,
                    floor,
                    Arc::clone(&spotter),
                    Arc::clone(&incidents),
                ))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self {
            tape,
            detector,
            cameras,
            incidents,
            desk,
            incoming,
        })
    }
}

/// Starts the agent on each incident as the cameras raise it.
pub fn dispatch(
    desk: Arc<agent::Desk>,
    mut incoming: tokio::sync::mpsc::UnboundedReceiver<watch::Incident>,
) {
    tokio::spawn(async move {
        while let Some(incident) = incoming.recv().await {
            if let Err(err) = desk.start(&incident).await {
                eprintln!("[{}] the agent could not start: {err}", incident.id);
            }
        }
    });
}

async fn run(common: Common, cards: CardMode) -> anyhow::Result<()> {
    let started = Instant::now();
    let App {
        tape,
        detector,
        cameras,
        incidents,
        desk,
        incoming,
    } = App::build(&common).await?;
    let desk = Arc::new(desk);
    println!(
        "watching {} for {} of the shift ({})",
        data::CAMERAS.join(", "),
        common
            .seconds
            .map_or_else(|| "all five minutes".into(), |s| format!("{s} s")),
        if tape.mode == Mode::Live {
            "live: Claude's answers are recorded"
        } else {
            "replaying recorded answers"
        }
    );
    desk.kernel.start().await?;
    let cancel = CancellationToken::new();
    tokio::spawn(terminal::narrate(
        desk.sessions.clone(),
        Arc::clone(&incidents),
        cancel.clone(),
    ));
    tokio::spawn(terminal::cards(
        desk.approvals.clone(),
        cards,
        cancel.clone(),
    ));
    dispatch(Arc::clone(&desk), incoming);

    let root = root();
    let plays = cameras.iter().map(|camera| {
        watch::play(
            &root,
            Arc::clone(camera),
            Arc::clone(&detector),
            None,
            common.seconds,
        )
    });
    let summaries = futures::future::join_all(plays).await;
    for (camera, summary) in cameras.iter().zip(summaries) {
        let summary = summary?;
        println!(
            "[camera] {}: {} frames observed, {} spots",
            camera.name,
            summary.observations,
            camera.spots()
        );
    }
    // The last spots and the agent's last reviews settle.
    let settled = settle(&desk, &incidents, Duration::from_secs(600)).await;
    if !settled {
        println!("[run] some reviews were still running after 10 minutes");
    }
    cancel.cancel();
    desk.kernel.stop().await?;
    tape.save()?;

    report(&common, &tape, &desk, &incidents, started.elapsed()).await
}

/// Waits until every incident's review has ended or is held for the supervisor.
async fn settle(desk: &agent::Desk, incidents: &Incidents, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    tokio::time::sleep(Duration::from_secs(2)).await;
    while Instant::now() < deadline {
        let mut open = 0;
        for incident in incidents.all() {
            if !desk.settled(&incident.id).await {
                open += 1;
            }
        }
        if open == 0 {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

async fn report(
    common: &Common,
    tape: &Tape,
    desk: &agent::Desk,
    incidents: &Incidents,
    took: Duration,
) -> anyhow::Result<()> {
    let root = root();
    let truth = data::Truth::load(&root)?;
    let until_ms = common.seconds.map_or(300_000, |s| s * 1_000);
    let events = eval::events(&truth, until_ms);
    let all = incidents.all();
    let mut raised = Vec::new();
    let mut decisions = BTreeMap::new();
    for incident in &all {
        let decided = desk.decision(&incident.id).await;
        if decided.as_deref() == Some("raise_incident") {
            raised.push(incident.clone());
        }
        decisions.insert(incident.id.clone(), decided);
    }
    let found = |list: &[watch::Incident]| -> Vec<eval::Found> {
        list.iter()
            .map(|i| eval::Found {
                id: i.id.clone(),
                camera: i.camera.clone(),
                at_ms: i.at_ms,
                x: i.person_floor_m[0],
                y: i.person_floor_m[1],
            })
            .collect()
    };
    let watched = eval::score(&truth, &events, &found(&all));
    let reviewed = eval::score(&truth, &events, &found(&raised));
    let spend = tape.spend();

    println!();
    println!(
        "results ({} of video, {} cameras, in {:.0?})",
        clock(until_ms),
        data::CAMERAS.len(),
        took
    );
    println!(
        "  ground-truth near-misses       {} (a moving vehicle within {:.1} m of a person, both in view)",
        events.len(),
        eval::TRUTH_GAP_M
    );
    println!(
        "  close approaches on the floor  {} → found {} of {} near-misses (recall {:.2}), {} on a near-miss (precision {:.2})",
        watched.reported,
        watched.events_found,
        watched.truth_events,
        watched.recall,
        watched.reports_on_an_event,
        watched.precision
    );
    println!(
        "    of the {} a camera shows clearly    found {} (recall {:.2})",
        watched.clear_events, watched.clear_found, watched.clear_recall
    );
    println!(
        "  raised by the agent            {} → found {} of {} (recall {:.2}), {} on a near-miss (precision {:.2})",
        reviewed.reported,
        reviewed.events_found,
        reviewed.truth_events,
        reviewed.recall,
        reviewed.reports_on_an_event,
        reviewed.precision
    );
    println!(
        "    of the {} a camera shows clearly    found {} (recall {:.2})",
        reviewed.clear_events, reviewed.clear_found, reviewed.clear_recall
    );
    let logged = decisions
        .values()
        .filter(|d| d.as_deref() == Some("shift_log"))
        .count();
    println!("  dismissed into the shift log   {logged}");
    for (role, s) in &spend {
        println!(
            "  {role:<6} {:<18} {:>4} calls {:>9} in {:>7} out  ${:.4}",
            s.model, s.calls, s.input_tokens, s.output_tokens, s.usd
        );
    }
    println!(
        "  total model spend              ${:.4}{}",
        spend.values().map(|s| s.usd).sum::<f64>(),
        if tape.mode == Mode::Replay {
            " (the recorded live run's; this replay spent nothing)"
        } else {
            ""
        }
    );
    let unseen: HashSet<_> = events
        .iter()
        .enumerate()
        .filter(|(i, _)| !watched.matches.values().any(|m| *m == Some(*i)))
        .map(|(_, e)| format!("{}–{} {:?}", clock(e.start_ms), clock(e.end_ms), e.cameras))
        .collect();
    println!("  near-misses no camera flagged  {}", unseen.len());

    std::fs::create_dir_all(&common.out)?;
    let results = json!({
        "videoMs": until_ms,
        "cameras": data::CAMERAS,
        "mode": if tape.mode == Mode::Live { "live" } else { "replay" },
        "truth": events,
        "incidents": all,
        "decisions": decisions,
        "watched": watched,
        "raised": reviewed,
        "spend": spend,
    });
    let path = common.out.join("results.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&results)?)?;
    println!("  results                        {}", path.display());
    println!(
        "  shift log                      {}",
        common.out.join("shift-log.md").display()
    );
    Ok(())
}
