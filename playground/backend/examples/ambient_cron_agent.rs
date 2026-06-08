//! Ambient Cron Agent Example
//!
//! Demonstrates **Ambient Agents** with a CronTrigger and real LLM integration.
//! The agent wraps a Gemini-powered motivational quote generator with lifecycle
//! control (start → pause → resume → stop).
//!
//! The CronTrigger fires every 2 seconds for demonstration purposes. The ambient
//! agent lifecycle is exercised fully:
//!   - Start the ambient agent → status: Running
//!   - Observe trigger events firing (3-4 triggers)
//!   - Pause → status: Paused (triggers are buffered)
//!   - Resume → status: Running (triggers resume)
//!   - Stop → status: Stopped
//!
//! Note: The current `AmbientAgent` implementation logs trigger events but does
//! not yet invoke the agent through a Runner (that requires a Runner reference).
//! This example demonstrates the event source pattern and lifecycle management.
//!
//! # Usage
//!
//! ```bash
//! cargo run --manifest-path examples/ambient_cron_agent/Cargo.toml
//! ```
//!
//! No API key is required since the ambient agent currently logs trigger events
//! without invoking the LLM. If `GOOGLE_API_KEY` is set, the example notes that
//! the agent *would* be invoked in a full production setup.

use std::sync::Arc;

use adk_agent::{AmbientAgent, AmbientAgentStatus, CronTrigger, LlmAgentBuilder};
use adk_core::Agent;
use adk_model::GeminiModel;
use tracing_subscriber::EnvFilter;

// ─── Constants ───────────────────────────────────────────────────────────────

const MODEL_NAME: &str = "gemini-2.5-flash";

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn api_key() -> Option<String> {
    std::env::var("GOOGLE_API_KEY")
        .ok()
        .or_else(|| std::env::var("GEMINI_API_KEY").ok())
        .filter(|key| !key.trim().is_empty())
}

fn print_banner() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║       Ambient Cron Agent — Event-Driven Background Agent     ║");
    println!("║                                                              ║");
    println!("║  Demonstrates: CronTrigger, AmbientAgent lifecycle           ║");
    println!("║  Pattern: start → observe → pause → resume → stop            ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
}

fn print_section(title: &str) {
    println!("\n┌─────────────────────────────────────────────────────────────┐");
    println!("│ {title:<60}│");
    println!("└─────────────────────────────────────────────────────────────┘");
}

fn status_emoji(status: AmbientAgentStatus) -> &'static str {
    match status {
        AmbientAgentStatus::Running => "🟢",
        AmbientAgentStatus::Paused => "🟡",
        AmbientAgentStatus::Stopped => "🔴",
    }
}

// ─── Entry Point ─────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    print_banner();

    // ─── Step 1: Create the underlying LlmAgent ──────────────────────────
    print_section("Step 1: Creating Gemini-powered quote agent");

    let has_key = api_key().is_some();

    // The AmbientAgent needs an Arc<dyn Agent>. We build a real Gemini agent
    // if a key is available; otherwise use a dummy key (the agent is never actually
    // invoked via Runner in this demo — only triggers fire).
    let key = api_key().unwrap_or_else(|| "not-set".to_string());

    if has_key {
        println!("  🔑 GOOGLE_API_KEY detected — building real Gemini agent");
        println!("  📡 Model: {MODEL_NAME}");
    } else {
        println!("  ℹ️  No GOOGLE_API_KEY set — using placeholder key for agent creation");
        println!("  💡 Set GOOGLE_API_KEY to enable real LLM invocations");
        println!("  💡 The trigger/lifecycle demo works without a key");
    }

    let model = Arc::new(GeminiModel::new(&key, MODEL_NAME)?);
    let agent: Arc<dyn Agent> = Arc::new(
        LlmAgentBuilder::new("motivational-quote-generator")
            .model(model)
            .instruction(
                "You are a motivational quote generator. Each time you are invoked, \
                 respond with a single unique inspirational quote. Keep it brief — \
                 one to two sentences maximum. Do not repeat quotes.",
            )
            .build()?,
    );

    println!("  ✓ Agent created: \"{}\"", agent.name());

    // ─── Step 2: Create the CronTrigger ──────────────────────────────────
    print_section("Step 2: Creating CronTrigger (every 2 seconds)");

    let trigger = CronTrigger::new("*/2 * * * * *")?;
    println!("  ✓ CronTrigger created: \"*/2 * * * * *\" (fires every 2 seconds)");
    println!("  📋 In production: \"0 9 * * *\" (daily at 9 AM), \"0 */6 * * *\" (every 6h)");

    // ─── Step 3: Create AmbientAgent ─────────────────────────────────────
    print_section("Step 3: Creating AmbientAgent");

    let mut ambient = AmbientAgent::new(agent, Arc::new(trigger));
    let status = ambient.status().await;
    println!(
        "  ✓ AmbientAgent created (initial status: {} {:?})",
        status_emoji(status),
        status
    );

    // ─── Step 4: Start — observe triggers ────────────────────────────────
    print_section("Step 4: Starting ambient agent (observe ~3 triggers)");

    ambient.start().await?;
    let status = ambient.status().await;
    println!(
        "  ✓ Agent started (status: {} {:?})",
        status_emoji(status),
        status
    );

    if has_key {
        println!("  📡 With a real model, each trigger would invoke the LLM");
    } else {
        println!("  📋 Trigger events are logged (check tracing output above)");
    }

    println!("  ⏳ Sleeping 7 seconds to observe triggers...");
    tokio::time::sleep(tokio::time::Duration::from_secs(7)).await;
    println!("  ✓ ~3 triggers should have fired (check INFO logs above)");

    // ─── Step 5: Pause ───────────────────────────────────────────────────
    print_section("Step 5: Pausing ambient agent");

    ambient.pause().await?;
    let status = ambient.status().await;
    println!(
        "  ✓ Agent paused (status: {} {:?})",
        status_emoji(status),
        status
    );
    println!("  📋 Subscription alive but events are buffered, not processed");

    println!("  ⏳ Sleeping 4 seconds while paused (no triggers processed)...");
    tokio::time::sleep(tokio::time::Duration::from_secs(4)).await;
    println!("  ✓ No triggers processed while paused");

    // ─── Step 6: Resume ──────────────────────────────────────────────────
    print_section("Step 6: Resuming ambient agent");

    ambient.resume().await?;
    let status = ambient.status().await;
    println!(
        "  ✓ Agent resumed (status: {} {:?})",
        status_emoji(status),
        status
    );

    println!("  ⏳ Sleeping 5 seconds to observe resumed triggers...");
    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    println!("  ✓ ~2 more triggers should have fired");

    // ─── Step 7: Stop ────────────────────────────────────────────────────
    print_section("Step 7: Stopping ambient agent");

    ambient.stop().await?;
    let status = ambient.status().await;
    println!(
        "  ✓ Agent stopped (status: {} {:?})",
        status_emoji(status),
        status
    );
    println!("  📋 Background task cancelled, resources cleaned up");

    // ─── Summary ─────────────────────────────────────────────────────────
    print_section("Lifecycle Summary");

    println!("  ┌──────────────────────────────────────────────────────┐");
    println!("  │  🔴 Stopped  →  start()  →  🟢 Running              │");
    println!("  │  🟢 Running  →  pause()  →  🟡 Paused               │");
    println!("  │  🟡 Paused   →  resume() →  🟢 Running              │");
    println!("  │  🟢 Running  →  stop()   →  🔴 Stopped              │");
    println!("  └──────────────────────────────────────────────────────┘");
    println!();
    println!("  Ambient agents are ideal for:");
    println!("    • Scheduled tasks (cron-based reports, digests, alerts)");
    println!("    • Event-driven processing (webhooks, file watchers)");
    println!("    • Background monitoring and automation");
    println!();
    println!("  Available triggers:");
    println!("    • CronTrigger     — time-based scheduling");
    println!("    • WebhookTrigger  — HTTP POST events");
    println!("    • FileWatchTrigger — filesystem change events");
    println!();

    Ok(())
}
