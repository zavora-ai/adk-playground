use adk_core::{
    AdkIdentity, AppName, ExecutionIdentity, IdentityError, InvocationId, SessionId, UserId,
};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_session::{AppendEventRequest, Event, GetRequest, InMemorySessionService};
use std::collections::HashMap;
use std::sync::Arc;

// ── Typed Identity — Injection-Proof Multi-Tenant Safety ──
// ADK-Rust validates ALL identity values at construction time.
// No raw strings ever reach the session or agent layer.
//
// Security guarantees:
//   - Empty values rejected → no accidental wildcard matches
//   - Null bytes rejected → no C-string truncation attacks
//   - Length capped at 512 bytes → no buffer overflow vectors
//   - Multi-tenant isolation → events scoped to exact identity
//
// This example:
//   1. Demonstrates injection attack prevention
//   2. Shows multi-tenant session isolation
//   3. Runs an agent scoped to a validated identity

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== 🔐 Typed Identity — Injection-Proof Multi-Tenant Safety ===\n");

    // ── 1. Validated identifier construction ──
    println!("── 1. Validated Identifier Construction ──\n");

    let app = AppName::try_from("secure-app")?;
    let user = UserId::try_from("tenant:alice@example.com")?;
    let session = SessionId::try_from("session-abc-123")?;
    let invocation = InvocationId::generate();

    println!("  AppName:      {}", app.as_ref());
    println!("  UserId:       {}", user.as_ref());
    println!("  SessionId:    {}", session.as_ref());
    println!("  InvocationId: {invocation}");

    let identity = AdkIdentity::new(app.clone(), user.clone(), session.clone());
    println!("  AdkIdentity:  {identity}");

    let exec = ExecutionIdentity {
        adk: identity.clone(),
        invocation_id: invocation,
        branch: "main".to_string(),
        agent_name: "planner".to_string(),
    };
    println!(
        "  ExecutionIdentity: agent={}, branch={}",
        exec.agent_name, exec.branch
    );

    // Serde round-trip
    let json = serde_json::to_string(&identity)?;
    let deser: AdkIdentity = serde_json::from_str(&json)?;
    assert_eq!(identity, deser);
    println!("  ✓ Serde round-trip: transparent JSON serialization\n");

    // ── 2. Injection attack prevention ──
    println!("── 2. Injection Attack Prevention ──\n");

    let attacks: Vec<(&str, &str, std::result::Result<(), IdentityError>)> = vec![
        ("Empty string", "\"\"", AppName::try_from("").map(|_| ())),
        (
            "Null byte",
            "\"bad\\0name\"",
            AppName::try_from("bad\0name").map(|_| ()),
        ),
        (
            "Overflow (513 bytes)",
            "\"x\" × 513",
            AppName::try_from("x".repeat(513).as_str()).map(|_| ()),
        ),
    ];

    for (name, input, result) in &attacks {
        match result {
            Ok(_) => println!("  ❌ {} ({}) — unexpectedly accepted", name, input),
            Err(e) => println!("  ✓ {} blocked: {}", name, e),
        }
    }
    assert!(AppName::try_from("a".repeat(512).as_str()).is_ok());
    println!("  ✓ Max length (512 bytes) accepted\n");

    // ── 3. Multi-tenant session isolation ──
    println!("── 3. Multi-Tenant Session Isolation ──\n");

    let service = InMemorySessionService::new();
    let shared_sid = "shared-session-42";

    // Two users, same session ID
    service
        .create(CreateRequest {
            app_name: "secure-app".into(),
            user_id: "alice".into(),
            session_id: Some(shared_sid.into()),
            state: HashMap::new(),
        })
        .await?;
    service
        .create(CreateRequest {
            app_name: "secure-app".into(),
            user_id: "bob".into(),
            session_id: Some(shared_sid.into()),
            state: HashMap::new(),
        })
        .await?;

    // Append event ONLY to Alice via typed identity
    let alice_id = AdkIdentity::new(
        AppName::try_from("secure-app")?,
        UserId::try_from("alice")?,
        SessionId::try_from(shared_sid)?,
    );
    service
        .append_event_for_identity(AppendEventRequest {
            identity: alice_id,
            event: Event::new("inv-alice"),
        })
        .await?;

    let alice_events = service
        .get(GetRequest {
            app_name: "secure-app".into(),
            user_id: "alice".into(),
            session_id: shared_sid.into(),
            num_recent_events: None,
            after: None,
        })
        .await?
        .events()
        .len();

    let bob_events = service
        .get(GetRequest {
            app_name: "secure-app".into(),
            user_id: "bob".into(),
            session_id: shared_sid.into(),
            num_recent_events: None,
            after: None,
        })
        .await?
        .events()
        .len();

    println!("  Same session ID '{shared_sid}' for two users:");
    println!("  Alice's events: {alice_events} ← received the event");
    println!("  Bob's events:   {bob_events} ← isolated, zero cross-contamination");
    assert_eq!(alice_events, 1);
    assert_eq!(bob_events, 0);
    println!("  ✓ Multi-tenant isolation confirmed\n");

    // ── 4. Agent scoped to validated identity ──
    println!("── 4. Agent Scoped to Validated Identity ──\n");

    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("identity_agent")
            .instruction(
                "You are a security-aware assistant. The user's identity has been \
                 validated at the boundary — no raw strings reach you. Explain briefly \
                 why typed identity prevents injection attacks in multi-tenant AI systems.",
            )
            .model(model)
            .build()?,
    );

    // Use validated identity types for the runner
    let validated_user = UserId::try_from("alice")?;
    let validated_session = SessionId::try_from("identity-demo")?;

    let sessions: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());
    sessions
        .create(CreateRequest {
            app_name: "playground".into(),
            user_id: validated_user.as_ref().to_string(),
            session_id: Some(validated_session.as_ref().to_string()),
            state: HashMap::new(),
        })
        .await?;

    let runner = Runner::new(RunnerConfig {
        app_name: "playground".into(),
        agent,
        session_service: sessions,
        artifact_service: None,
        memory_service: None,
        plugin_manager: None,
        run_config: None,
        compaction_config: None,
        context_cache_config: None,
        cache_capable: None,
        request_context: None,
        cancellation_token: None,
    })?;

    let message = Content::new("user").with_text(
        "How does typed identity (validated AppName, UserId, SessionId) prevent \
         injection attacks in multi-tenant AI agent systems? Be concise.",
    );
    let mut stream = runner
        .run(validated_user, validated_session, message)
        .await?;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() {
                    print!("{text}");
                }
            }
        }
    }
    println!("\n");

    println!("=== All typed identity security checks passed! ===");
    Ok(())
}
