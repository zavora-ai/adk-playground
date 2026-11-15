//! Gemini Managed Agents — Practical Examples with Streaming
//!
//! Four focused agents that do real work, with live SSE streaming so you can
//! watch the agent think, execute code, and produce output in real time.
//!
//! Based on: https://ai.google.dev/gemini-api/docs/managed-agents-quickstart
//!
//! # Usage
//!
//! ```bash
//! export GOOGLE_API_KEY=your-key-here
//! cargo run -p gemini-managed-agents             # run all agents
//! cargo run -p gemini-managed-agents -- code     # code agent only
//! cargo run -p gemini-managed-agents -- research # research agent only
//! cargo run -p gemini-managed-agents -- dev      # multi-turn dev agent
//! cargo run -p gemini-managed-agents -- custom   # saved agent CRUD
//! ```

use std::env;
use std::time::{Duration, Instant};

use adk_gemini::Gemini;
use adk_gemini::interactions::{
    Environment, EnvironmentConfig, EnvironmentSource, InteractionSseEvent,
    NetworkConfig, NetworkRule, StepDelta,
};
use futures::StreamExt;
use tracing_subscriber::EnvFilter;

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn api_key() -> Option<String> {
    env::var("GOOGLE_API_KEY")
        .ok()
        .or_else(|| env::var("GEMINI_API_KEY").ok())
        .filter(|k| !k.trim().is_empty())
}

fn banner(title: &str) {
    let line = "=".repeat(60);
    println!("\n{line}");
    println!("  {title}");
    println!("{line}\n");
}

/// Maximum time to wait for a single interaction before cancelling.
const TIMEOUT: Duration = Duration::from_secs(120);

/// Stream an interaction, printing live progress. Returns the final output text.
///
/// This demonstrates the recommended pattern from the official docs:
/// - Watch `step.start` to see what the agent is doing (thinking, code exec, search)
/// - Accumulate `step.delta` text fragments for the final output
/// - Cancel if the agent runs longer than the timeout
async fn stream_interaction(
    gemini: &Gemini,
    builder: adk_gemini::interactions::InteractionBuilder,
) -> anyhow::Result<String> {
    let start = Instant::now();
    let mut stream = builder.stream().await?;

    let mut output = String::new();
    let mut interaction_id: Option<String> = None;
    #[allow(unused_assignments)]
    let mut current_step_type = String::new();
    let mut step_count = 0u32;
    let mut token_total = 0i64;

    while let Some(event) = stream.next().await {
        // Check timeout
        if start.elapsed() > TIMEOUT {
            if let Some(id) = &interaction_id {
                eprintln!("\n⏱️  Timeout ({TIMEOUT:?}) — cancelling interaction {id}...");
                if let Err(e) = gemini.cancel_interaction(id).await {
                    eprintln!("   Cancel failed: {e}");
                }
            }
            break;
        }

        let event = match event {
            Ok(e) => e,
            Err(e) => {
                eprintln!("\n⚠️  Stream error: {e}");
                break;
            }
        };

        match event {
            InteractionSseEvent::InteractionCreated { interaction, .. } => {
                interaction_id = Some(interaction.id.clone());
                println!("🆔 Interaction: {}", interaction.id);
                if let Some(env_id) = &interaction.environment_id {
                    println!("📦 Environment: {env_id}");
                }
                println!("⏳ Status: {:?}\n", interaction.status);
            }

            InteractionSseEvent::StepStart { index, step, .. } => {
                step_count += 1;
                // Extract step type from the Step enum
                current_step_type = match &step {
                    adk_gemini::interactions::Step::Thought { .. } => "💭 thinking".to_string(),
                    adk_gemini::interactions::Step::ModelOutput { .. } => {
                        "📝 writing output".to_string()
                    }
                    adk_gemini::interactions::Step::FunctionCall { name, .. } => {
                        format!("🔧 calling {name}")
                    }
                    adk_gemini::interactions::Step::Other(v) => {
                        // Server-side tools: code_execution_call, google_search_call, etc.
                        let t = v
                            .get("type")
                            .and_then(|t| t.as_str())
                            .unwrap_or("unknown");
                        match t {
                            "code_execution_call" => "💻 executing code".to_string(),
                            "code_execution_result" => "📋 code result".to_string(),
                            "google_search_call" => "🔍 searching web".to_string(),
                            "google_search_result" => "📰 search results".to_string(),
                            "url_context_call" => "🌐 fetching URL".to_string(),
                            "url_context_result" => "📄 URL content".to_string(),
                            other => format!("⚙️  {other}"),
                        }
                    }
                    _ => "▶️  step".to_string(),
                };
                print!("  [{index}] {current_step_type}");
                // Flush without newline so deltas appear on same line
                use std::io::Write;
                std::io::stdout().flush().ok();
            }

            InteractionSseEvent::StepDelta { delta, .. } => match &delta {
                StepDelta::Text { text } => {
                    output.push_str(text);
                    // Show a dot for each text chunk to indicate progress
                    print!(".");
                    use std::io::Write;
                    std::io::stdout().flush().ok();
                }
                StepDelta::Other(v) => {
                    // Handle thought_summary, thought_signature, etc.
                    let delta_type = v
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("unknown");
                    if delta_type == "thought_summary" {
                        print!(".");
                        use std::io::Write;
                        std::io::stdout().flush().ok();
                    }
                    // Silently skip signatures and other internal deltas
                }
                _ => {}
            },

            InteractionSseEvent::StepStop { .. } => {
                println!(" ✓");
            }

            InteractionSseEvent::InteractionCompleted { interaction, .. } => {
                if let Some(usage) = &interaction.usage {
                    token_total = usage.total_tokens;
                }
                println!(
                    "\n✅ Completed in {:.1}s | {step_count} steps | {token_total} tokens",
                    start.elapsed().as_secs_f64()
                );
            }

            InteractionSseEvent::InteractionStatusUpdate { status, .. } => {
                if status.is_terminal() && status != adk_gemini::interactions::InteractionStatus::Completed {
                    println!("\n⚠️  Status: {status:?}");
                }
            }

            InteractionSseEvent::Error { error, .. } => {
                eprintln!("\n❌ Error: {} (code: {:?})", error.message, error.code);
                anyhow::bail!("Interaction failed: {}", error.message);
            }

            InteractionSseEvent::Other(_) => {
                // Gracefully skip unknown event types per the docs
            }
        }
    }

    Ok(output)
}

// ─────────────────────────────────────────────────────────────────────────────
// Agent 1: Code Agent
//
// Official quickstart equivalent: "Run your first agent interaction"
// The agent writes code, executes it in the sandbox, and returns results.
// ─────────────────────────────────────────────────────────────────────────────

async fn agent_code(gemini: &Gemini) -> anyhow::Result<()> {
    banner("Agent 1: Code Agent — Write & Execute in Sandbox");

    println!("Task: Generate Fibonacci numbers, save to file, print contents\n");

    let builder = gemini
        .create_interaction()
        .antigravity()
        .environment(Environment::remote())
        .input_text(
            "Write a Python script that generates the first 20 Fibonacci numbers \
             and saves them to fibonacci.txt. Then read the file and print its contents.",
        );

    let output = stream_interaction(gemini, builder).await?;

    println!("\n--- Final Output ---");
    println!("{output}");
    println!("--- End ---");

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Agent 2: Research Agent
//
// Uses built-in google_search + url_context to research and summarize.
// ─────────────────────────────────────────────────────────────────────────────

async fn agent_research(gemini: &Gemini) -> anyhow::Result<()> {
    banner("Agent 2: Research Agent — Search & Summarize");

    println!("Task: Read Hacker News, summarize top 5 stories\n");

    let builder = gemini
        .create_interaction()
        .antigravity()
        .environment(Environment::remote())
        .input_text(
            "Read Hacker News (https://news.ycombinator.com), summarize the top 5 \
             stories today with title, URL, and a one-sentence summary for each. \
             Save the results as summary.md in the workspace.",
        );

    let output = stream_interaction(gemini, builder).await?;

    println!("\n--- Final Output ---");
    println!("{output}");
    println!("--- End ---");

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Agent 3: Multi-Turn Dev Agent
//
// Official quickstart equivalent: "Continue the conversation (multi-turn)"
// Two independent state dimensions:
//   - Conversation context (previous_interaction_id)
//   - Environment state (environment ID)
// ─────────────────────────────────────────────────────────────────────────────

async fn agent_dev(gemini: &Gemini) -> anyhow::Result<()> {
    banner("Agent 3: Multi-Turn Dev — Iterative Coding with Sandbox Resume");

    // Turn 1: scaffold a project
    println!("── Turn 1: Create a Rust library ──\n");

    let turn1 = gemini
        .create_interaction()
        .antigravity()
        .environment(Environment::remote())
        .input_text(
            "Create a Rust library project called 'string_utils' with `cargo init --lib`. \
             Implement `pub fn reverse_words(s: &str) -> String` that reverses word order. \
             Run `cargo check` to verify it compiles.",
        )
        .send()
        .await?;

    let env_id = turn1
        .environment_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("No environment_id returned"))?;

    println!("✅ Turn 1 complete");
    println!("   Interaction: {}", turn1.id);
    println!("   Environment: {env_id}");
    println!(
        "   Output: {}...",
        turn1
            .output_text()
            .unwrap_or_default()
            .chars()
            .take(150)
            .collect::<String>()
    );

    // Turn 2: resume sandbox + conversation, add tests and run them
    println!("\n── Turn 2: Add tests (same sandbox, same conversation) ──\n");

    let builder = gemini
        .create_interaction()
        .antigravity()
        .environment(Environment::resume(env_id))
        .previous_interaction_id(turn1.id.clone())
        .input_text(
            "Add unit tests for reverse_words covering: empty string, single word, \
             multiple words, extra whitespace. Run `cargo test` and show results.",
        );

    let output = stream_interaction(gemini, builder).await?;

    println!("\n--- Turn 2 Output ---");
    println!("{output}");
    println!("--- End ---");

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Agent 4: Custom Saved Agent
//
// Official quickstart equivalent: "Save a managed agent" + "Invoke the managed agent"
// Full CRUD lifecycle: create → list → invoke → delete
// ─────────────────────────────────────────────────────────────────────────────

async fn agent_custom(gemini: &Gemini) -> anyhow::Result<()> {
    banner("Agent 4: Custom Saved Agent — CRUD + Invoke");

    let agent_id = "adk-rust-code-reviewer";

    // Create with base environment (AGENTS.md for instructions)
    println!("📝 Creating saved agent '{agent_id}'...");

    let base_env = EnvironmentConfig::new()
        .with_sources(vec![EnvironmentSource::Inline {
            content: "# Code Review Agent\n\n\
                      For every code snippet:\n\
                      1. Check correctness and edge cases\n\
                      2. Suggest performance improvements\n\
                      3. Flag security issues\n\
                      4. Rate 1-10 with justification\n\
                      \n\
                      Be constructive, specific, and concise."
                .to_string(),
            target: ".agents/AGENTS.md".to_string(),
        }])
        .with_network(NetworkConfig::allowlist(vec![
            NetworkRule::new("crates.io"),
            NetworkRule::new("docs.rs"),
        ]));

    let saved = gemini
        .create_agent()
        .id(agent_id)
        .base_agent("antigravity-preview-05-2026")
        .system_instruction(
            "You are a senior code reviewer. Follow .agents/AGENTS.md strictly.",
        )
        .base_environment(base_env)
        .build_and_save()
        .await?;

    println!("   ✓ Created: {:?}", saved.id);

    // List
    let list = gemini.list_agents().await?;
    println!(
        "   📋 Agents on account: [{}]",
        list.agents
            .iter()
            .filter_map(|a| a.id.as_deref())
            .collect::<Vec<_>>()
            .join(", ")
    );

    // Invoke
    println!("\n🚀 Invoking saved agent for code review...\n");

    let builder = gemini
        .create_interaction()
        .agent(agent_id)
        .store(true)
        .environment(Environment::remote())
        .input_text(
            "Review this Rust function:\n\n\
             ```rust\n\
             fn find_duplicates(items: Vec<String>) -> Vec<String> {\n\
                 let mut seen = Vec::new();\n\
                 let mut dupes = Vec::new();\n\
                 for item in items {\n\
                     if seen.contains(&item) {\n\
                         dupes.push(item.clone());\n\
                     } else {\n\
                         seen.push(item);\n\
                     }\n\
                 }\n\
                 dupes\n\
             }\n\
             ```",
        );

    let output = stream_interaction(gemini, builder).await?;

    println!("\n--- Code Review ---");
    println!("{output}");
    println!("--- End ---");

    // Cleanup
    println!("\n🗑️  Deleting agent '{agent_id}'...");
    gemini.delete_agent(agent_id).await?;
    println!("   ✓ Deleted.");

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();

    println!("Gemini Managed Agents — Practical Examples");
    println!("==========================================");
    println!("Streaming enabled: watch the agent work in real time.");
    println!("Timeout: {TIMEOUT:?} per interaction (auto-cancels if stuck).\n");

    let Some(key) = api_key() else {
        eprintln!(
            "ERROR: No API key found.\n\n\
             Set GOOGLE_API_KEY or GEMINI_API_KEY in your environment.\n\
             The key needs Interactions API (Beta) access.\n\n\
             See .env.example for details."
        );
        std::process::exit(1);
    };

    let gemini = Gemini::new(&key)?;

    let args: Vec<String> = env::args().collect();
    let filter = args.get(1).map(|s| s.as_str());

    match filter {
        Some("code") => agent_code(&gemini).await?,
        Some("research") => agent_research(&gemini).await?,
        Some("dev") | Some("multiturn") => agent_dev(&gemini).await?,
        Some("custom") => agent_custom(&gemini).await?,
        Some("all") | None => {
            agent_code(&gemini).await?;
            agent_research(&gemini).await?;
            agent_dev(&gemini).await?;
            agent_custom(&gemini).await?;
        }
        Some(other) => {
            eprintln!("Unknown agent: '{other}'");
            eprintln!("Available: code, research, dev, custom, all");
            std::process::exit(1);
        }
    }

    println!("\n✓ Done.");
    Ok(())
}
