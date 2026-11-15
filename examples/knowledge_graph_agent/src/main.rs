//! # Knowledge-graph memory for an ordinary text agent
//!
//! A plain [`LlmAgent`](adk_agent::LlmAgent) — no realtime, no voice — given a
//! **bi-temporal knowledge graph** as its long-term memory, using the reusable
//! `adk-tool` built-ins:
//!
//! - [`GraphMemoryToolset`](adk_tool::GraphMemoryToolset) — the agent's
//!   `remember` / `relate` write tools (feature `graph-memory-tools`).
//! - [`LoadMemoryTool`](adk_tool::memory::LoadMemoryTool) — recall, which works
//!   over any [`MemoryService`](adk_memory::MemoryService); here that's the
//!   [`GraphMemoryService`].
//!
//! All three are backed by one shared [`GraphMemoryService`]. The point: this is
//! exactly how *any* agent wires up KG memory — nothing here is bespoke to a
//! voice/realtime stack.
//!
//! ## What it demonstrates
//!
//! 1. **Session 1** — the agent meets "Alex", and as Alex shares durable facts
//!    it calls `remember`/`relate` to write them into the graph.
//! 2. We print the resulting graph (entities, observations, relations).
//! 3. **Session 2** — a brand-new session with its *own* `SessionService` (so it
//!    shares **no** chat history with session 1) still knows everything about
//!    Alex, because the profile card is injected into its instruction and
//!    `load_memory` can recall the rest — all from the shared graph.
//!
//! ## Run
//!
//! ```bash
//! export GOOGLE_API_KEY=...   # or GEMINI_API_KEY
//! cargo run --manifest-path examples/knowledge_graph_agent/Cargo.toml
//! ```
//!
//! The graph here is in-memory (`sqlite::memory:`) and shared across both
//! sessions in one process. Point it at a file (`sqlite:kg.db`) to have the
//! agent remember the user across process restarts.

use std::collections::HashMap;
use std::sync::Arc;

use adk_agent::LlmAgentBuilder;
use adk_core::{Agent, Content, Part, SessionId, UserId};
use adk_memory::GraphMemoryService;
use adk_model::GeminiModel;
use adk_runner::Runner;
use adk_session::{CreateRequest, InMemorySessionService, SessionService};
use adk_tool::GraphMemoryToolset;
use adk_tool::memory::LoadMemoryTool;
use futures::StreamExt;

const APP_NAME: &str = "kg-text-agent";
const USER_ID: &str = "alex";
const DEFAULT_MODEL: &str = "gemini-2.5-flash";

const INSTRUCTION: &str = "You are a personal assistant with durable long-term memory backed by a \
knowledge graph. When the user shares a stable fact about themselves — their name, a preference, \
a constraint such as a diet or allergy, a goal, their employer, or a relationship — save it with \
the `remember` tool, and use `relate` to connect entities (for example, the user works_at a \
company). When a question depends on what you know about the user, call `load_memory` to recall it \
and ground your answer in what you find. Save facts quietly without repeating them back verbatim. \
Keep replies to one or two sentences.";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let Some(key) = api_key() else {
        eprintln!(
            "This example needs a Gemini API key. Set GOOGLE_API_KEY (or GEMINI_API_KEY) and re-run:\n  \
             export GOOGLE_API_KEY=your-key\n  \
             cargo run --manifest-path examples/knowledge_graph_agent/Cargo.toml"
        );
        return Ok(());
    };
    let model_name = std::env::var("GEMINI_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    let model = Arc::new(GeminiModel::new(&key, &model_name)?);

    // One graph, shared by both sessions (and by all three tools).
    let kg = Arc::new(GraphMemoryService::new("sqlite::memory:").await?);
    kg.migrate().await?;

    // ── Session 1: the agent learns about Alex ───────────────────────────────
    separator("Session 1 — the agent meets Alex and learns about them");
    let agent = build_agent(model.clone(), kg.clone(), "assistant-1").await?;
    let runner = make_runner(agent, "session-1").await?;
    run_turn(
        &runner,
        "session-1",
        "Hi! I'm Alex. I'm vegetarian and severely allergic to peanuts.",
    )
    .await?;
    run_turn(
        &runner,
        "session-1",
        "I work at Acme Corp as a data engineer, and I'm training for a marathon in April.",
    )
    .await?;
    run_turn(&runner, "session-1", "What have you noted about me so far?").await?;

    separator("Knowledge graph after session 1");
    print_graph(&kg).await?;

    // ── Session 2: a fresh session recalls everything from the graph ──────────
    separator("Session 2 — a brand-new session (no shared chat history) still knows Alex");
    let agent = build_agent(model.clone(), kg.clone(), "assistant-2").await?;
    let runner = make_runner(agent, "session-2").await?;
    run_turn(
        &runner,
        "session-2",
        "I'm ordering dinner — suggest a dish for me, keeping my needs in mind.",
    )
    .await?;
    run_turn(&runner, "session-2", "Also, remind me what I'm training for and where I work.")
        .await?;

    separator("How this worked");
    println!(
        "Session 2 ran on its own SessionService — it shared no conversation history\n\
         with session 1. Everything it knew about Alex came from the shared knowledge\n\
         graph: the `remember`/`relate` writes from session 1, surfaced via the profile\n\
         card injected into session 2's instruction and the `load_memory` recall tool."
    );
    Ok(())
}

/// Read the Gemini API key, preferring `GOOGLE_API_KEY`.
fn api_key() -> Option<String> {
    std::env::var("GOOGLE_API_KEY")
        .ok()
        .or_else(|| std::env::var("GEMINI_API_KEY").ok())
        .filter(|k| !k.trim().is_empty())
}

/// Build an `LlmAgent` wired to the shared graph: `remember`/`relate` for
/// writes (via [`GraphMemoryToolset`]) and `load_memory` for recall. The graph's
/// current profile card is injected into the instruction, so the agent starts
/// each session already knowing the user.
async fn build_agent(
    model: Arc<GeminiModel>,
    kg: Arc<GraphMemoryService>,
    name: &str,
) -> anyhow::Result<Arc<dyn Agent>> {
    let card = kg.profile_card(APP_NAME, USER_ID).await?;
    let instruction = if card.trim().is_empty() {
        INSTRUCTION.to_string()
    } else {
        format!("{INSTRUCTION}\n\n{card}")
    };

    let load_memory =
        LoadMemoryTool::builder().memory_service(kg.clone()).max_results(20).build()?;

    let agent = LlmAgentBuilder::new(name)
        .model(model)
        .instruction(instruction)
        .toolset(Arc::new(GraphMemoryToolset::new(kg.clone()))) // remember + relate
        .tool(Arc::new(load_memory)) // recall
        .build()?;
    Ok(Arc::new(agent))
}

/// A `Runner` backed by a **fresh** in-memory session, created up front. Each
/// call gets its own `SessionService`, so different sessions share no chat
/// history — only the knowledge graph.
async fn make_runner(agent: Arc<dyn Agent>, session_id: &str) -> anyhow::Result<Runner> {
    let sessions: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());
    sessions
        .create(CreateRequest {
            app_name: APP_NAME.into(),
            user_id: USER_ID.into(),
            session_id: Some(session_id.into()),
            state: HashMap::new(),
        })
        .await?;
    Ok(Runner::builder().app_name(APP_NAME).agent(agent).session_service(sessions).build()?)
}

/// Drive one turn, streaming the reply and surfacing every tool round-trip so
/// the `remember`/`relate`/`load_memory` calls are visible.
async fn run_turn(runner: &Runner, session_id: &str, prompt: &str) -> anyhow::Result<()> {
    println!("\n👤 {prompt}");
    print!("🤖 ");
    let mut stream = runner
        .run(
            UserId::new(USER_ID)?,
            SessionId::new(session_id)?,
            Content::new("user").with_text(prompt),
        )
        .await?;

    while let Some(event) = stream.next().await {
        let event = event?;
        let Some(content) = &event.llm_response.content else { continue };
        for part in &content.parts {
            match part {
                Part::Text { text } if !text.trim().is_empty() => print!("{text}"),
                Part::FunctionCall { name, args, .. } => println!("\n   🛠  {name}({args})"),
                Part::FunctionResponse { function_response, .. } => {
                    println!("   ↩  {} → {}", function_response.name, function_response.response)
                }
                _ => {}
            }
        }
    }
    println!();
    Ok(())
}

/// Print the user's whole graph: entities with their current observations, then
/// relations.
async fn print_graph(kg: &GraphMemoryService) -> anyhow::Result<()> {
    let (entities, relations) = kg.read_graph(APP_NAME, USER_ID).await?;
    if entities.is_empty() {
        println!("   (empty)");
        return Ok(());
    }
    for e in &entities {
        println!("   • {} [{}]", e.name, e.entity_type);
        for o in &e.observations {
            println!("       – {}", o.content);
        }
    }
    if !relations.is_empty() {
        println!("   relations:");
        for r in &relations {
            println!("       {} —{}→ {}", r.source, r.relation_type, r.target);
        }
    }
    Ok(())
}

fn separator(title: &str) {
    let line = "═".repeat(74);
    println!("\n{line}\n  {title}\n{line}");
}
