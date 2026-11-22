//! Desk Watch: an agent that watches a desk through a camera on this computer.
//!
//! | Command | What runs |
//! |---------|-----------|
//! | `sources` | The cameras ffmpeg lists on this machine |
//! | `watch` | Headless: the agent sets up watches from what you ask, sleeps, and wakes on each event |
//! | `serve` | The web home's live camera page over the same pipeline, the agent beside it |
//!
//! Frames stay in memory. RF-DETR Nano runs on this computer; Claude sees a frame only through
//! `vision_describe`, after the egress gate has covered every person and blanked personal text.

mod desk;
mod serve;
mod terminal;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use adk_core::audit_log::{AuditActor, AuditTrail, InMemoryAuditRecorder};
use adk_core::{BudgetTracker, ModelCallScope, NewRun, RunBudget, UsageUnit};
use adk_kernel::{ProcessKind, ProcessSpec};
use adk_vision::stream::{
    DeviceSource, FrameBudget, FrameKeeper, FrameSources, NoRedaction, SourceSpec,
};
use adk_vision::{
    ByteTracker, DetectQuery, Detector, FrameSourceId, MediaScope, Sampling, Tracker, VisionCx,
    VisionRuntime,
};
use clap::{Args, Parser, Subcommand};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::desk::{APP, Desk, LABELS, SESSION, STREAM, Settings, USER};
use crate::terminal::CardMode;

#[derive(Parser)]
#[command(name = "vision-live-desk", about = "An agent that watches your desk through a camera")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Lists the cameras ffmpeg finds on this machine.
    Sources,
    /// Watches headless: the agent sets up watches from `--ask`, then wakes on each event.
    Watch {
        #[command(flatten)]
        common: Common,
        /// What you want the agent to watch for.
        #[arg(
            long,
            default_value = "Watch my desk. Tell me when a person sits at the desk, and when a cell phone or a cup appears on it."
        )]
        ask: String,
        /// What the terminal does with a held card.
        #[arg(long, value_enum, default_value_t = CardMode::Ask)]
        cards: CardMode,
        /// Stops by itself after this many minutes; Ctrl-C stops it any time.
        #[arg(long)]
        minutes: Option<u64>,
    },
    /// Serves the web home's live camera page over the same pipeline.
    Serve {
        #[command(flatten)]
        common: Common,
        /// The port on 127.0.0.1.
        #[arg(long, default_value_t = 8815)]
        port: u16,
    },
}

#[derive(Args, Clone)]
struct Common {
    /// A camera index or name from `sources`, or `testsrc` for ffmpeg's test pattern.
    #[arg(long, default_value = "0")]
    device: String,
    /// The capture size, one the camera offers.
    #[arg(long, default_value = "1280x720")]
    video_size: String,
    /// Frames the detector looks at each second.
    #[arg(long, default_value_t = 2.0)]
    fps: f32,
    /// Frames the governor lets the detector see each UTC day.
    #[arg(long, default_value_t = 50_000)]
    daily_frames: u64,
    /// The detector's score threshold.
    #[arg(long, default_value_t = 0.45)]
    threshold: f32,
    /// The desk's zone in sampled-frame pixels, `x0,y0,x1,y1`; the lower two thirds by default.
    #[arg(long)]
    zone: Option<String>,
    /// The harness model.
    #[arg(long, default_value = "claude-sonnet-5-5")]
    model: String,
    /// The model `vision_describe` asks, through the egress gate.
    #[arg(long, default_value = "claude-haiku-5-5")]
    describer_model: String,
    /// What the egress gate does with a describe request: allow (after covering people and
    /// personal text), ask, or deny.
    #[arg(long, default_value = "allow")]
    egress: String,
    /// The most the kernel spends on model calls each UTC day, in US dollars.
    #[arg(long, default_value_t = 2.0)]
    daily_usd: f64,
    /// The folder the desk log and kept snapshots go in.
    #[arg(long, default_value = "desk")]
    dir: PathBuf,
}

impl Common {
    fn settings(&self) -> anyhow::Result<Settings> {
        let (w, h) = self
            .video_size
            .split_once('x')
            .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
            .ok_or_else(|| anyhow::anyhow!("--video-size is WIDTHxHEIGHT, such as 1280x720"))?;
        Ok(Settings {
            device: self.device.clone(),
            video_size: Some((w, h)),
            fps: self.fps,
            max_long_edge: 640,
            daily_frames: self.daily_frames,
            threshold: self.threshold,
            model: self.model.clone(),
            describer_model: self.describer_model.clone(),
            egress: self.egress.clone(),
            daily_usd: self.daily_usd,
            dir: self.dir.clone(),
        })
    }

    /// The desk zone's corners in sampled-frame pixels.
    fn zone(&self, settings: &Settings) -> anyhow::Result<[f64; 4]> {
        let (w, h) = settings.frame_size();
        match &self.zone {
            None => Ok([0.0, (f64::from(h) / 3.0).round(), f64::from(w), f64::from(h)]),
            Some(text) => {
                let values: Vec<f64> =
                    text.split(',').map(|v| v.trim().parse()).collect::<Result<_, _>>()?;
                let [x0, y0, x1, y1] = values[..] else {
                    anyhow::bail!("--zone is x0,y0,x1,y1");
                };
                Ok([x0, y0, x1, y1])
            }
        }
    }
}

fn banner(settings: &Settings, mode: &str) {
    let (w, h) = settings.frame_size();
    println!("Desk Watch — {mode}");
    println!("  camera      {} (sampled at {} fps, {w} × {h})", settings.device, settings.fps);
    println!("  on device   RF-DETR Nano + ByteTrack; frames stay in memory");
    println!("  agent       {} (harness, Auto mode, kernel-hosted)", settings.model);
    println!(
        "  leaves      only vision_describe frames, to {} ({}), egress rule `{}`, people and personal text covered first",
        desk::ANTHROPIC_HOST,
        settings.describer_model,
        settings.egress
    );
    println!("  log         {}", settings.log_path().display());
    println!("  daily caps  {} frames, ${:.2} of model calls", settings.daily_frames, settings.daily_usd);
    println!();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    adk_core::ensure_crypto_provider();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();
    match Cli::parse().command {
        Command::Sources => sources().await,
        Command::Watch { common, ask, cards, minutes } => watch(common, ask, cards, minutes).await,
        Command::Serve { common, port } => {
            let settings = common.settings()?;
            let zone = common.zone(&settings)?;
            serve::serve(settings, zone, port).await
        }
    }
}

async fn sources() -> anyhow::Result<()> {
    let devices = tokio::task::spawn_blocking(DeviceSource::list).await??;
    if devices.is_empty() {
        println!("ffmpeg lists no camera on this machine; `--device testsrc` uses its test pattern");
    }
    for device in devices {
        println!("{:>3}  {:<7} {}", device.index, format!("{:?}", device.kind).to_lowercase(), device.name);
    }
    Ok(())
}

async fn watch(common: Common, ask: String, cards: CardMode, minutes: Option<u64>) -> anyhow::Result<()> {
    let settings = common.settings()?;
    let [x0, y0, x1, y1] = common.zone(&settings)?;
    banner(&settings, "headless");
    println!("loading RF-DETR Nano (fetched and checked on first run)…");
    let detector = desk::rf_detr(settings.threshold).await?;

    let audit = InMemoryAuditRecorder::new();
    let frames = Arc::new(BudgetTracker::new(RunBudget::new()));
    let streams = FrameSources::new(
        VisionCx::new()
            .with_scope(ModelCallScope::new().with_budget(frames.clone()))
            .with_audit(AuditTrail::new(Arc::new(audit.clone())), AuditActor::default()),
    );
    let zone = format!("[[{x0},{y0}],[{x1},{y0}],[{x1},{y1}],[{x0},{y1}]]");
    let desk = Desk::build(
        settings.clone(),
        detector.clone(),
        audit.clone(),
        Some(streams.clone()),
        format!(
            "The desk surface is the zone {zone}: use it, with zoneAnchor \"bottomCenter\", for \
             conditions about a person at the desk or things on it."
        ),
    )
    .await?;

    let stream_detector: Arc<dyn Detector> = detector.clone();
    let stream_runtime = Arc::new(VisionRuntime::builder().detector(stream_detector).build()?);
    // Wake frames are kept in the session the woken runs use, where the vision tools load them.
    let keeper = Arc::new(FrameKeeper::new(
        desk.artifacts.clone(),
        MediaScope::new(APP, USER, SESSION),
        FrameSourceId(STREAM.into()),
    ));
    streams.register(
        SourceSpec::new(desk::camera(&settings), Arc::new(NoRedaction), keeper, desk.book.clone())
            .describe("The camera on the person's desk, looking at the desk and the chair in front of it")
            .detector(stream_runtime, DetectQuery::Classes(LABELS.iter().map(|l| (*l).to_string()).collect()))
            .tracker(Arc::new(|| Box::new(ByteTracker::default()) as Box<dyn Tracker>))
            .sampling(Sampling { fps: settings.fps, max_long_edge: Some(settings.max_long_edge) })
            .budget(
                FrameBudget::new(settings.fps)
                    .with_min_local_fps((settings.fps / 4.0).min(0.5))
                    .with_local_quota(settings.daily_frames),
            ),
    )?;

    desk.kernel.start().await?;
    let cancel = CancellationToken::new();
    tokio::spawn(terminal::narrate(desk.sessions.clone(), cancel.clone()));
    tokio::spawn(terminal::cards(desk.approvals.clone(), cards, cancel.clone()));

    println!("[you] {ask}");
    desk.kernel
        .processes()
        .spawn(
            NewRun::new("interactive", APP, USER, APP, json!({ "text": ask })).with_session_id(SESSION),
            ProcessSpec::new(ProcessKind::Interactive),
        )
        .await?;

    let stop = async {
        match minutes {
            Some(minutes) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    () = tokio::time::sleep(Duration::from_secs(minutes * 60)) => {}
                }
            }
            None => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    };
    stop.await;
    println!();
    println!("stopping: the camera first, then the kernel…");
    streams.shutdown().await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    cancel.cancel();
    desk.kernel.stop().await?;

    let records = audit.records();
    let count = |action: &str| records.iter().filter(|r| r.action == action).count();
    println!();
    println!("summary");
    println!("  frames metered        {}", frames.units().get(UsageUnit::Frames));
    println!("  watches registered    {}", count("vision.watch.registered"));
    println!("  watch events          {}", count("vision.watch.fired"));
    println!("  frames kept (memory)  {}", count("vision.evidence.kept"));
    println!(
        "  egress records        {}",
        records.iter().filter(|r| r.action.starts_with("vision.egress")).count()
    );
    println!("  desk log              {}", settings.log_path().display());
    for status in streams.statuses() {
        println!("  source                {status:?}");
    }
    Ok(())
}
