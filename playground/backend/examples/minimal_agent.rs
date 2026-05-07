use adk_rust::prelude::*;
use adk_rust::run;

// ── Minimal Agent — v0.8 Lightweight Tier ──
// Demonstrates the v0.8 minimal feature tier:
//
// With `adk-rust = "0.8.0"` (no extra features), you get:
//   - Gemini model
//   - LlmAgent
//   - Runner
//   - InMemorySessionService
//   - `adk::run()` one-liner
//
// That's it. No CLI, no server, no tools, no telemetry.
// The result: 32% lighter builds and ~50s compile times.
//
// The simplest possible ADK program is a single function call:
//   `adk_rust::run(instructions, input).await?`
//
// For the full production preset, add: features = ["standard"]

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== Minimal Agent — Lightweight v0.8 Tier ===\n");

    // ── Part 1: Show what's available in minimal tier ──
    println!("── Part 1: Minimal Tier Components ──\n");
    println!("  ✅ GeminiModel — Google's Gemini API");
    println!("  ✅ LlmAgentBuilder — build agents with instructions");
    println!("  ✅ Runner — execute agents with streaming");
    println!("  ✅ InMemorySessionService — ephemeral sessions");
    println!("  ✅ adk_rust::run() — single-function agent invocation");
    println!("  ❌ CLI Launcher — requires 'cli' feature");
    println!("  ❌ HTTP Server — requires 'server' feature");
    println!("  ❌ OpenAI/Anthropic — requires 'openai'/'anthropic' features");
    println!("  ❌ Telemetry — requires 'telemetry' feature");
    println!("  ❌ Memory — requires 'memory' feature");
    println!("\n  📦 Result: ~32% smaller binary, ~50s compile time");

    // ── Part 2: The simplest possible agent — one function call ──
    println!("\n── Part 2: One-Liner Agent (adk::run) ──\n");
    println!("  Code: adk_rust::run(instructions, input).await?");
    println!("  - Auto-detects GOOGLE_API_KEY from environment");
    println!("  - Creates ephemeral session, runs agent, returns text\n");

    let response = run(
        "You are a concise assistant. Answer in exactly one sentence.",
        "What is Rust's ownership model?",
    )
    .await?;

    println!("  Q: What is Rust's ownership model?");
    println!("  A: {}", response);

    // ── Part 3: Three quick questions ──
    println!("\n── Part 3: Multiple Invocations ──\n");

    let questions = [
        ("What does 'zero-cost abstractions' mean?", "zero-cost abstractions"),
        ("Name one advantage of async/await.", "async/await"),
        ("What is a trait in Rust?", "traits"),
    ];

    for (q, topic) in &questions {
        let answer = run(
            "You are a Rust expert. Answer in one concise sentence.",
            q,
        )
        .await?;
        println!("  [{topic}] Q: {q}");
        println!("           A: {answer}\n");
    }

    // ── Part 4: Feature tier comparison ──
    println!("── Part 4: Feature Tier Comparison ──\n");
    println!("  ┌─────────────┬──────────────────────────────────────────────────┬──────────────────────┐");
    println!("  │ Tier        │ Includes                                         │ Use Case             │");
    println!("  ├─────────────┼──────────────────────────────────────────────────┼──────────────────────┤");
    println!("  │ minimal     │ Gemini, agents, runner, sessions                 │ Fast starter agents  │");
    println!("  │ standard    │ + OpenAI, Anthropic, tools, memory, telemetry... │ Production deploy    │");
    println!("  │ enterprise  │ + realtime, browser, RAG, payments, AWP          │ Full-featured prod   │");
    println!("  │ full        │ + audio, code, sandbox                           │ Everything           │");
    println!("  └─────────────┴──────────────────────────────────────────────────┴──────────────────────┘");

    println!("\n=== Key Features ===");
    println!("• adk_rust::run(instructions, input) — simplest possible agent");
    println!("• adk-rust = \"0.8.0\" — minimal tier by default (32% lighter)");
    println!("• features = [\"standard\"] — restore full production preset");
    println!("• Perfect for serverless, edge, or embedded deployments");
    println!("• Compile time: ~50s (minimal) vs ~2min (full)");
    Ok(())
}
