use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_session::{EncryptedSession, EncryptionKey};
use std::collections::HashMap;
use std::sync::Arc;

// ── Auto-Provider Detection + Encrypted Sessions ──
// Two competitive features in one agentic demo:
//
// 1. `provider_from_env()` — zero-config model selection. Detects which API key
//    is set (ANTHROPIC → OPENAI → GOOGLE) and returns the right client. No more
//    hardcoding provider constructors.
//
// 2. `EncryptedSession` — AES-256-GCM encryption wrapping any SessionService.
//    State is encrypted at rest. Transparent to the agent — it reads/writes
//    normally while the middleware handles encrypt/decrypt.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== Auto-Provider + Encrypted Sessions ===\n");

    // ── Part 1: Auto-detect provider from environment ──
    println!("── Part 1: provider_from_env() ──\n");

    let model = match adk_rust::provider_from_env() {
        Ok(m) => {
            println!("✅ Auto-detected provider from environment");
            m
        }
        Err(e) => {
            println!("❌ {}", e);
            println!("\nSet one of: ANTHROPIC_API_KEY, OPENAI_API_KEY, or GOOGLE_API_KEY");
            return Ok(());
        }
    };

    // ── Part 2: Encrypted session storage ──
    println!("\n── Part 2: Encrypted Session Storage ──\n");

    let key = EncryptionKey::generate();
    println!("🔐 Generated AES-256-GCM encryption key ({} bytes)", key.as_bytes().len());

    let inner = InMemorySessionService::new();
    let sessions: Arc<dyn SessionService> = Arc::new(EncryptedSession::new(inner, key, vec![]));

    // Pre-populate session with "sensitive" state
    let mut initial_state = HashMap::new();
    initial_state.insert("user_name".to_string(), serde_json::json!("Alice"));
    initial_state.insert("clearance".to_string(), serde_json::json!("top-secret"));
    initial_state.insert("preferences".to_string(), serde_json::json!({
        "language": "en",
        "timezone": "UTC-5"
    }));

    let uid = UserId::new("user")?;
    let sid = SessionId::new("encrypted-s1")?;

    sessions
        .create(CreateRequest {
            app_name: "playground".into(),
            user_id: uid.to_string(),
            session_id: Some(sid.to_string()),
            state: initial_state,
        })
        .await?;
    println!("📦 Created encrypted session with user state");
    println!("   State is AES-256-GCM encrypted at rest — transparent to the agent\n");

    // ── Part 3: Run agent with auto-detected provider + encrypted session ──
    println!("── Part 3: Agent with Auto-Provider + Encrypted State ──\n");

    let agent = Arc::new(
        LlmAgentBuilder::new("secure_assistant")
            .instruction(
                "You are a secure assistant. The user's name and clearance level are \
                 stored in the encrypted session. Greet the user by name and acknowledge \
                 their clearance level. Be concise — 2-3 sentences max.",
            )
            .model(model)
            .build()?,
    );

    let runner = Runner::new(RunnerConfig {
        app_name: "playground".into(),
        agent,
        session_service: sessions.clone(),
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

    let message = Content::new("user").with_text("Hello, who am I and what's my access level?");
    let mut stream = runner.run(uid.clone(), sid.clone(), message).await?;

    print!("🤖 ");
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() {
                    print!("{}", text);
                }
            }
        }
    }

    // Verify session state survived encrypted round-trip
    println!("\n\n── Verification ──\n");
    let session = sessions
        .get(adk_rust::session::GetRequest {
            app_name: "playground".into(),
            user_id: uid.to_string(),
            session_id: sid.to_string(),
            num_recent_events: None,
            after: None,
        })
        .await?;

    let state = session.state().all();
    println!("🔓 Decrypted session state:");
    println!("   user_name:  {}", state.get("user_name").unwrap_or(&serde_json::json!("?")));
    println!("   clearance:  {}", state.get("clearance").unwrap_or(&serde_json::json!("?")));
    println!("   preferences: {}", state.get("preferences").unwrap_or(&serde_json::json!("?")));

    println!("\n=== Key Features ===");
    println!("• provider_from_env() — one function, any provider, zero config");
    println!("• EncryptedSession — AES-256-GCM, transparent to agents");
    println!("• EncryptionKey::generate() — cryptographically random 32-byte keys");
    println!("• Supports key rotation with old_keys fallback list");
    Ok(())
}
