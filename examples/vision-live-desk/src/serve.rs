//! The web home with the live camera page over the desk camera's frame pipeline: overlays,
//! tracks, the zone and its watches, the governor, the switch, and raising a card from a frame.
//! Watch events wake the same kernel-hosted agent, whose held cards show beside the camera.

use std::sync::Arc;
use std::time::Duration;

use adk_artifact::ArtifactService;
use adk_core::audit_log::{AuditActor, AuditTrail, InMemoryAuditRecorder};
use adk_core::{RequestContext, SingleAgentLoader};
use adk_server::auth_bridge::{RequestContextError, RequestContextExtractor};
use adk_server::{ServerBuilder, ServerConfig};
use adk_surface::camera::{CameraFacts, Ingest, LiveCamera, LiveWatch, PipelineCameras, ZoneFacts};
use adk_surface::{CardHint, CardHints, LocalSource, Place, SurfaceEngine};
use adk_vision::stream::{FrameBudget, NoRedaction};
use adk_vision::{
    ByteTracker, DetectQuery, Detector, Sampling, VisionCx, VisionRuntime, WatchCondition,
};
use async_trait::async_trait;

use crate::desk::{self, APP, Desk, LABELS, SESSION, STREAM, Settings, USER};

/// Admits every request as the desk's owner, presence confirmed: the server listens on
/// loopback only.
struct Owner;

#[async_trait]
impl RequestContextExtractor for Owner {
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

/// The heading a desk card shows.
struct DeskCards;

#[async_trait]
impl CardHints for DeskCards {
    async fn hint(&self, request: &adk_core::ApprovalRequest) -> Option<CardHint> {
        (request.tool_name == "raise_card").then(|| {
            let title = request.args_redacted["title"].as_str().unwrap_or("Look at the desk");
            CardHint::new().with_heading(title.to_string())
        })
    }
}

fn watch(id: &str, name: &str, label: &str, wakes_for: &str) -> LiveWatch {
    LiveWatch::new(
        id,
        name,
        WatchCondition::new(id, [label])
            .with_min_score(0.5)
            .with_min_frames(3)
            .with_cooldown(Duration::from_secs(90)),
    )
    .in_zone("desk")
    .wakes_for(wakes_for)
}

pub async fn serve(settings: Settings, zone: [f64; 4], port: u16) -> anyhow::Result<()> {
    crate::banner(&settings, "web home");
    println!("loading RF-DETR Nano (fetched and checked on first run)…");
    let detector = desk::rf_detr(settings.threshold).await?;
    let audit = InMemoryAuditRecorder::new();
    let desk = Desk::build(
        settings.clone(),
        detector.clone(),
        audit.clone(),
        None,
        "The person watches the camera on the web home and edits its zone and watches there. \
         A card they raise from the camera page names the frame by digest: describe that frame \
         once, log one line, and answer them; do not raise another card for it."
            .to_string(),
    )
    .await?;

    let (w, h) = settings.frame_size();
    let stream_detector: Arc<dyn Detector> = detector.clone();
    let runtime = Arc::new(VisionRuntime::builder().detector(stream_detector).build()?);
    let camera = LiveCamera::stream(
        CameraFacts::new(STREAM, "Desk camera", APP, SESSION)
            .with_place("the desk")
            .passing("things on the desk")
            .with_ingest(Ingest {
                kind: "USB camera".into(),
                width: w,
                height: h,
                at: "this computer".into(),
            }),
        desk::camera(&settings),
        Arc::new(NoRedaction),
    )
    .detector(runtime, DetectQuery::Classes(LABELS.iter().map(|l| (*l).to_string()).collect()))
    .tracker(|| Box::new(ByteTracker::default()))
    .sampling(Sampling { fps: settings.fps, max_long_edge: Some(settings.max_long_edge) })
    .budget(
        FrameBudget::new(settings.fps)
            .with_min_local_fps((settings.fps / 4.0).min(0.5))
            .with_local_quota(settings.daily_frames),
    )
    .zone(ZoneFacts { id: "desk".into(), name: "Desk".into(), corners: zone })
    .watch(watch("person-at-desk", "Person at the desk", "person", "someone at the desk"))
    .watch(watch("phone-on-desk", "Phone on the desk", "cell phone", "a phone on the desk"))
    .watch(watch("cup-on-desk", "Cup on the desk", "cup", "a cup on the desk"));
    let live = PipelineCameras::builder(APP, USER)
        .artifacts(desk.artifacts.clone() as Arc<dyn ArtifactService>)
        .cx(VisionCx::new())
        .audit(AuditTrail::new(Arc::new(audit.clone())), AuditActor::default())
        .wakes(desk.book.clone())
        .camera(camera)
        .build()?;
    let cameras = live.start();

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
        .agent_name(APP, "Desk Watch")
        .card_hints(Arc::new(DeskCards))
        .cameras(cameras.clone())
        .build();
    let config = ServerConfig::new(Arc::new(SingleAgentLoader::new(desk.agent.clone())), desk.sessions.clone())
        .with_request_context(Arc::new(Owner));
    let app = ServerBuilder::new(config)
        .add_api_routes(adk_surface::web::router(engine))
        .add_root_routes(adk_surface::web::home())
        .build();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("live camera page on http://127.0.0.1:{port}/ui/cameras/{STREAM}");
    println!("the agent's thread on http://127.0.0.1:{port}/ui/  (Ctrl-C stops the camera, then the server)");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    println!("stopping…");
    drop(cameras);
    drop(live);
    desk.kernel.stop().await?;
    let records = audit.records();
    let count = |action: &str| records.iter().filter(|r| r.action == action).count();
    println!(
        "summary: {} watch events, {} frames kept in memory, {} camera acts recorded",
        count("vision.watch.fired"),
        count("vision.evidence.kept"),
        records.iter().filter(|r| r.action.starts_with("surface.camera")).count()
    );
    Ok(())
}
