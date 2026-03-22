//! Plugin System — lifecycle hooks that wrap a real agent run
//!
//! Creates plugins with before/after callbacks, composes them into a
//! PluginManager, and runs an LLM agent with the plugin pipeline active.
//! Watch the lifecycle events fire around the actual LLM call.

use adk_rust::prelude::*;
use adk_rust::session::{SessionService, CreateRequest};
use adk_rust::futures::StreamExt;
use adk_core::{UserId, SessionId};
use adk_plugin::{Plugin, PluginBuilder, PluginConfig, PluginManager};
use adk_core::callbacks::BeforeModelResult;
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== Plugin System — Live Agent Lifecycle ===\n");

    // ── 1. Logging plugin ──
    let logging = Plugin::new(PluginConfig {
        name: "logging".to_string(),
        on_user_message: Some(Box::new(|_ctx, content| {
            Box::pin(async move {
                let text = content.parts.iter()
                    .filter_map(|p| p.text())
                    .collect::<Vec<_>>().join(" ");
                println!("  📝 [log] User message: \"{}\"", &text[..text.len().min(60)]);
                Ok(None)
            })
        })),
        on_event: Some(Box::new(|_ctx, event| {
            Box::pin(async move {
                println!("  📝 [log] Event '{}' by {}", event.id, event.author);
                Ok(None)
            })
        })),
        ..Default::default()
    });
    println!("✓ Logging plugin registered (on_user_message + on_event)");

    // ── 2. Metrics plugin ──
    let metrics = Plugin::new(PluginConfig {
        name: "metrics".to_string(),
        before_run: Some(Box::new(|_ctx| {
            Box::pin(async move {
                println!("  📊 [metrics] Run starting...");
                Ok(None)
            })
        })),
        after_run: Some(Box::new(|_ctx| {
            Box::pin(async move {
                println!("  📊 [metrics] Run completed");
            })
        })),
        ..Default::default()
    });
    println!("✓ Metrics plugin registered (before_run + after_run)");

    // ── 3. Model interceptor via builder ──
    let interceptor = PluginBuilder::new("model-interceptor")
        .before_model(Box::new(|_ctx, request| {
            Box::pin(async move {
                println!("  🔍 [interceptor] LLM request intercepted — passing through");
                Ok(BeforeModelResult::Continue(request))
            })
        }))
        .after_model(Box::new(|_ctx, _response| {
            Box::pin(async move {
                println!("  🔍 [interceptor] LLM response received");
                Ok(None)
            })
        }))
        .build();
    println!("✓ Model interceptor registered (before_model + after_model)");

    // ── 4. Compose into PluginManager ──
    let manager = PluginManager::new(vec![logging, metrics, interceptor]);
    println!("✓ PluginManager: {} plugins composed\n", 3);

    // ── 5. Build agent with plugin manager ──
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("plugged_agent")
            .instruction("You are a helpful assistant. Be concise (2-3 sentences).")
            .model(model)
            .build()?
    );

    let sessions = Arc::new(InMemorySessionService::new());
    sessions.create(CreateRequest {
        app_name: "playground".into(),
        user_id: "user".into(),
        session_id: Some("s1".into()),
        state: HashMap::new(),
    }).await?;

    let runner = Runner::new(RunnerConfig {
        app_name: "playground".into(),
        agent,
        session_service: sessions,
        artifact_service: None,
        memory_service: None,
        plugin_manager: Some(Arc::new(manager)),
        run_config: None,
        compaction_config: None,
        context_cache_config: None,
        cache_capable: None,
        request_context: None,
        cancellation_token: None,
    })?;

    // ── 6. Run — watch lifecycle events fire ──
    println!("--- Running agent with plugins active ---\n");

    let message = Content::new("user")
        .with_text("What are the benefits of plugin architectures in software systems?");
    print!("**Agent:** ");
    let mut stream = runner.run(UserId::new("user")?, SessionId::new("s1")?, message).await?;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() { print!("{}", text); }
            }
        }
    }
    println!("\n");

    println!("--- Plugin lifecycle complete ---");
    Ok(())
}
