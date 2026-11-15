//! Gemini Interactions API — runtime agent example.
//!
//! Mirrors the ADK-Python `interactions_api` sample, driving the Interactions
//! API (Beta) as a **transport toggle on the Gemini model**. The same
//! `LlmAgent`, `Runner`, tool loop, and sessions are used unchanged — the only
//! difference is `GeminiModel::use_interactions_api(true)`.
//!
//! Scenarios:
//!   1. Basic generation — a normal `LlmAgent` using the Interactions transport,
//!      printing the response text and the server-assigned `interaction_id`.
//!   2. Google Search via bypass — `GoogleSearchTool` converted to a
//!      function-calling tool with `with_bypass_multi_tools_limit`, so it can be
//!      mixed with custom function tools (which the Interactions API otherwise
//!      forbids).
//!   3. Multi-turn stateful conversation — two turns on the same session,
//!      demonstrating server-side context retention and chained
//!      `interaction_id`s.
//!   4. Custom function tool alongside the bypassed search tool — the tool-mixing
//!      solution in action.
//!
//! All scenarios require a Gemini API key with Interactions API (Beta) access.
//! When no key is present the example prints guidance and exits cleanly.
//!
//! # Usage
//!
//! ```bash
//! export GOOGLE_API_KEY=your-key-here
//! cargo run --manifest-path examples/gemini_interactions_agent/Cargo.toml
//! # or, from the workspace root:
//! cargo run -p gemini-interactions-agent
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use adk_agent::LlmAgentBuilder;
use adk_core::{Agent, Content, Part, Result, SessionId, Tool, ToolContext, UserId};
use adk_model::GeminiModel;
use adk_runner::Runner;
use adk_session::{CreateRequest, InMemorySessionService, SessionService};
use adk_tool::{BypassMultiToolsLimit, FunctionTool, GoogleSearchTool};
use futures::StreamExt;
use serde_json::{Value, json};
use tracing_subscriber::EnvFilter;

const APP_NAME: &str = "gemini-interactions-agent";
const USER_ID: &str = "user";
/// A model on the Interactions API allowlist (see the spec's target table).
const MODEL_NAME: &str = "gemini-2.5-flash";

// ---------------------------------------------------------------------------
// Environment helpers
// ---------------------------------------------------------------------------

/// Reads the Gemini API key from the environment, preferring `GOOGLE_API_KEY`
/// and falling back to `GEMINI_API_KEY`.
fn api_key() -> Option<String> {
    std::env::var("GOOGLE_API_KEY")
        .ok()
        .or_else(|| std::env::var("GEMINI_API_KEY").ok())
        .filter(|key| !key.trim().is_empty())
}

fn separator(title: &str) {
    println!("\n{}", "=".repeat(64));
    println!("  {title}");
    println!("{}\n", "=".repeat(64));
}

// ---------------------------------------------------------------------------
// Custom function tool — a simple deterministic "unit converter".
// ---------------------------------------------------------------------------

#[derive(schemars::JsonSchema, serde::Serialize)]
struct ConvertArgs {
    /// The temperature in degrees Celsius to convert to Fahrenheit.
    celsius: f64,
}

/// Converts a Celsius temperature to Fahrenheit. Demonstrates a plain
/// function-calling tool coexisting with a bypassed built-in tool.
async fn celsius_to_fahrenheit(_ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
    let celsius = args.get("celsius").and_then(Value::as_f64).unwrap_or(0.0);
    let fahrenheit = celsius * 9.0 / 5.0 + 32.0;
    Ok(json!({ "celsius": celsius, "fahrenheit": fahrenheit }))
}

fn temperature_tool() -> Arc<dyn Tool> {
    Arc::new(
        FunctionTool::new(
            "celsius_to_fahrenheit",
            "Converts a temperature in degrees Celsius to degrees Fahrenheit.",
            celsius_to_fahrenheit,
        )
        .with_parameters_schema::<ConvertArgs>(),
    )
}

// ---------------------------------------------------------------------------
// Runner / agent plumbing
// ---------------------------------------------------------------------------

/// Builds a `GeminiModel` with the Interactions transport enabled.
fn interactions_model(api_key: &str) -> anyhow::Result<Arc<GeminiModel>> {
    Ok(Arc::new(
        GeminiModel::new(api_key, MODEL_NAME)?
            // Transport toggle — the agent/runner/tool loop are unchanged.
            .use_interactions_api(true)?,
    ))
}

/// Creates a `Runner` for `agent` backed by an in-memory session, and creates
/// the named session up front so multi-turn calls can reuse it.
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

/// Result of draining a single conversational turn.
#[derive(Debug, Default)]
struct TurnOutcome {
    text: String,
    interaction_id: Option<String>,
    function_calls: Vec<String>,
    function_responses: Vec<String>,
}

/// Drives one turn through the runner, printing streamed parts and collecting a
/// summary (text, first `interaction_id`, and any tool round-trips).
async fn run_turn(runner: &Runner, session_id: &str, prompt: &str) -> anyhow::Result<TurnOutcome> {
    println!("👤 {prompt}");

    let mut stream = runner
        .run(
            UserId::new(USER_ID)?,
            SessionId::new(session_id)?,
            Content::new("user").with_text(prompt),
        )
        .await?;

    let mut outcome = TurnOutcome::default();

    while let Some(event) = stream.next().await {
        let event = event?;

        // First-class interaction_id surfaced on the Event (ADK-Python parity
        // with `event.interaction_id`).
        if outcome.interaction_id.is_none()
            && let Some(id) = event.interaction_id()
        {
            outcome.interaction_id = Some(id.to_string());
        }

        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                match part {
                    Part::Text { text } if !text.trim().is_empty() => {
                        print!("{text}");
                        outcome.text.push_str(text);
                    }
                    Part::FunctionCall { name, args, .. } => {
                        println!("  → function_call: {name}({args})");
                        outcome.function_calls.push(name.clone());
                    }
                    Part::FunctionResponse { function_response, .. } => {
                        println!("  ← function_response: {}", function_response.response);
                        outcome.function_responses.push(function_response.name.clone());
                    }
                    Part::Thinking { .. } => {
                        println!("  💭 (thinking)");
                    }
                    _ => {}
                }
            }
        }
    }

    if !outcome.text.trim().is_empty() {
        println!();
    }
    if let Some(id) = &outcome.interaction_id {
        println!("🆔 interaction_id: {id}");
    }

    Ok(outcome)
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

/// Scenario 1: basic generation through the Interactions transport.
async fn scenario_basic_generation(api_key: &str) -> anyhow::Result<()> {
    separator("1. Basic generation (Interactions transport)");

    let model = interactions_model(api_key)?;
    let agent = Arc::new(
        LlmAgentBuilder::new("interactions-basic")
            .model(model)
            .instruction("You are a helpful assistant. Answer concisely.")
            .build()?,
    );

    let runner = make_runner(agent, "basic").await?;
    let outcome = run_turn(&runner, "basic", "Say hello in one short sentence.").await?;

    if outcome.text.trim().is_empty() {
        anyhow::bail!("expected the Interactions transport to produce text output");
    }
    Ok(())
}

/// Scenario 2: Google Search via `with_bypass_multi_tools_limit`.
///
/// The built-in `GoogleSearchTool` is converted into a function-calling tool by
/// delegating to an internal single-turn `LlmAgent` that performs grounded
/// search server-side. This is what lets it coexist with custom function tools
/// under the Interactions API.
async fn scenario_google_search_bypass(api_key: &str) -> anyhow::Result<()> {
    separator("2. Google Search via bypass_multi_tools_limit");

    // Internal grounded-search agent: a normal LlmAgent with the built-in
    // GoogleSearchTool and a Gemini model.
    let search_model = Arc::new(GeminiModel::new(api_key, MODEL_NAME)?);
    let search_agent: Arc<dyn Agent> = Arc::new(
        LlmAgentBuilder::new("google_search_agent")
            .model(search_model)
            .instruction("You perform grounded Google searches and return concise answers.")
            .tool(Arc::new(GoogleSearchTool::new()))
            .build()?,
    );

    // Convert the built-in tool into a uniform function-calling tool.
    let search_tool = GoogleSearchTool::new().with_bypass_multi_tools_limit(search_agent);
    println!("bypassed google_search.is_builtin() == {}", search_tool.is_builtin());

    let model = interactions_model(api_key)?;
    let agent = Arc::new(
        LlmAgentBuilder::new("interactions-search")
            .model(model)
            .instruction(
                "You are a research assistant. Use the google_search tool to find current \
                 information, then answer concisely.",
            )
            .tool(search_tool)
            .build()?,
    );

    let runner = make_runner(agent, "search").await?;
    run_turn(&runner, "search", "What is the latest stable version of Rust? Use google_search.")
        .await?;

    Ok(())
}

/// Scenario 3: multi-turn stateful conversation.
///
/// Two turns on the same persistent session. The session persists turn 1's
/// `Event` (carrying `interaction_id`), so the agent populates
/// `previous_response_id` on turn 2 and the transport chains statefully via
/// `previous_interaction_id`.
async fn scenario_multi_turn_stateful(api_key: &str) -> anyhow::Result<()> {
    separator("3. Multi-turn stateful conversation");

    let model = interactions_model(api_key)?;
    let agent = Arc::new(
        LlmAgentBuilder::new("interactions-stateful")
            .model(model)
            .instruction("You are a helpful assistant. Answer concisely.")
            .build()?,
    );

    let runner = make_runner(agent, "stateful").await?;

    // Turn 1: establish a fact.
    let turn1 = run_turn(&runner, "stateful", "My favorite color is blue. Remember it.").await?;
    // Turn 2 (same session): recall it.
    let turn2 = run_turn(&runner, "stateful", "What is my favorite color?").await?;

    match (&turn1.interaction_id, &turn2.interaction_id) {
        (Some(id1), Some(id2)) if id1 != id2 => {
            println!("\n🔗 chained interactions: {id1} → {id2}");
        }
        (Some(_), Some(_)) => {
            println!("\n⚠ both turns shared the same interaction_id (expected distinct ids)");
        }
        _ => {
            println!("\n⚠ one or both turns did not surface an interaction_id");
        }
    }

    if turn2.text.to_lowercase().contains("blue") {
        println!("✓ context retained: turn 2 recalled \"blue\"");
    } else {
        println!("⚠ turn 2 did not clearly recall the favorite color");
    }

    Ok(())
}

/// Scenario 4: a custom function tool alongside the bypassed search tool.
///
/// Demonstrates the tool-mixing solution: a bypass-converted built-in
/// (`google_search`) and a plain function tool (`celsius_to_fahrenheit`) in the
/// same agent. Both are uniform function tools, satisfying the Interactions
/// API's no-mixing restriction.
async fn scenario_mixed_tools(api_key: &str) -> anyhow::Result<()> {
    separator("4. Custom function tool + bypassed Google Search");

    let search_model = Arc::new(GeminiModel::new(api_key, MODEL_NAME)?);
    let search_agent: Arc<dyn Agent> = Arc::new(
        LlmAgentBuilder::new("google_search_agent")
            .model(search_model)
            .instruction("You perform grounded Google searches and return concise answers.")
            .tool(Arc::new(GoogleSearchTool::new()))
            .build()?,
    );
    let search_tool = GoogleSearchTool::new().with_bypass_multi_tools_limit(search_agent);

    let model = interactions_model(api_key)?;
    let agent = Arc::new(
        LlmAgentBuilder::new("interactions-mixed-tools")
            .model(model)
            .instruction(
                "You are an assistant with two tools: google_search for web lookups and \
                 celsius_to_fahrenheit for temperature conversion. Use whichever fits the \
                 request.",
            )
            .tool(search_tool)
            .tool(temperature_tool())
            .build()?,
    );

    let runner = make_runner(agent, "mixed").await?;
    let outcome = run_turn(
        &runner,
        "mixed",
        "Convert 25 degrees Celsius to Fahrenheit using the celsius_to_fahrenheit tool.",
    )
    .await?;

    if outcome.function_calls.is_empty() {
        println!("⚠ no function call observed — the model answered directly");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    println!("Gemini Interactions API — Runtime Agent Example");
    println!("===============================================");
    println!("Model: {MODEL_NAME} (Interactions transport)\n");

    let Some(api_key) = api_key() else {
        println!(
            "No API key found. Set GOOGLE_API_KEY (or GEMINI_API_KEY) to run this example.\n\
             The key must have Interactions API (Beta) access for the target model.\n\
             See .env.example for the expected variables."
        );
        return Ok(());
    };

    scenario_basic_generation(&api_key).await?;
    scenario_google_search_bypass(&api_key).await?;
    scenario_multi_turn_stateful(&api_key).await?;
    scenario_mixed_tools(&api_key).await?;

    separator("Done");
    println!("All scenarios completed.");
    Ok(())
}
