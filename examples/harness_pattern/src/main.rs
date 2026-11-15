//! # Harness Pattern — Trait-Based Agent Execution with Real LLM
//!
//! Demonstrates the core design pattern where:
//!
//! - **`Harness`** is the trait (abstract contract for running an agent)
//! - **`Runner`** is the production implementation (real Gemini API calls, timeout, streaming)
//! - **`TestHarness`** is a deterministic substitute (no API calls, canned responses)
//! - **`MyAgent`** depends ONLY on `Arc<dyn Harness>` — never the concrete type
//!
//! ## Run
//!
//! ```bash
//! cargo run --manifest-path examples/harness_pattern/Cargo.toml
//! ```
//!
//! Requires: `GOOGLE_API_KEY` environment variable.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use adk_core::{Content, Llm, LlmRequest, Part};
use adk_model::GeminiModel;
use async_trait::async_trait;
use futures::StreamExt;

// ═══════════════════════════════════════════════════════════════════════════════
// THE CONTRACT
// ═══════════════════════════════════════════════════════════════════════════════

/// The abstract execution contract.
///
/// Any struct that implements `Harness` can run an agent — whether it's a real
/// model client, a test double, a distributed executor, or a sandboxed runner.
#[async_trait]
pub trait Harness: Send + Sync {
    /// Execute and return the full response.
    async fn run(&self, input: AgentInput) -> Result<AgentOutput, HarnessError>;

    /// Execute with streaming output (token by token).
    async fn run_stream(&self, input: AgentInput) -> Result<AgentStream, HarnessError>;

    /// Human-readable name for logging.
    fn name(&self) -> &str;
}

// ═══════════════════════════════════════════════════════════════════════════════
// DOMAIN TYPES
// ═══════════════════════════════════════════════════════════════════════════════

pub struct AgentInput {
    pub session_id: String,
    pub message: String,
    pub context: HashMap<String, String>,
}

pub struct AgentOutput {
    pub session_id: String,
    pub response: String,
    pub tokens_used: u32,
}

pub type AgentStream = tokio::sync::mpsc::Receiver<String>;

#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    #[error("Model error: {0}")]
    Model(String),
    #[error("Timeout after {0}ms")]
    Timeout(u64),
    #[error("Session not found: {0}")]
    SessionNotFound(String),
}

// ═══════════════════════════════════════════════════════════════════════════════
// RUNNER: PRODUCTION IMPLEMENTATION (REAL LLM)
// ═══════════════════════════════════════════════════════════════════════════════

/// The production execution engine backed by a real LLM.
///
/// Uses `adk_core::Llm` trait — works with Gemini, OpenAI, Anthropic, etc.
pub struct Runner {
    name: String,
    llm: Arc<dyn Llm>,
    system_instruction: String,
    timeout_ms: u64,
}

impl Runner {
    pub fn new(name: impl Into<String>, llm: Arc<dyn Llm>) -> Self {
        Self {
            name: name.into(),
            llm,
            system_instruction: "You are a helpful assistant. Be concise.".into(),
            timeout_ms: 30_000,
        }
    }

    pub fn with_timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    pub fn with_system_instruction(mut self, instruction: impl Into<String>) -> Self {
        self.system_instruction = instruction.into();
        self
    }

    fn build_request(&self, message: &str) -> LlmRequest {
        let contents = vec![
            Content::new("user").with_text(message),
        ];
        let mut request = LlmRequest::new("gemini-2.5-flash", contents);
        // System instruction goes into config or as the first content with "system" role
        // For Gemini, we prepend it as a system content
        request.contents.insert(0, Content::new("system").with_text(&self.system_instruction));
        request
    }
}

#[async_trait]
impl Harness for Runner {
    async fn run(&self, input: AgentInput) -> Result<AgentOutput, HarnessError> {
        let request = self.build_request(&input.message);

        let mut response_stream = tokio::time::timeout(
            Duration::from_millis(self.timeout_ms),
            self.llm.generate_content(request, false),
        )
        .await
        .map_err(|_| HarnessError::Timeout(self.timeout_ms))?
        .map_err(|e| HarnessError::Model(e.to_string()))?;

        // Collect the full response from the stream
        let mut full_text = String::new();
        while let Some(chunk) = response_stream.next().await {
            match chunk {
                Ok(resp) => {
                    if let Some(content) = &resp.content {
                        for part in &content.parts {
                            if let Part::Text { text } = part {
                                full_text.push_str(text);
                            }
                        }
                    }
                }
                Err(e) => return Err(HarnessError::Model(e.to_string())),
            }
        }

        Ok(AgentOutput {
            session_id: input.session_id,
            response: full_text,
            tokens_used: 0, // Real token count available from response metadata
        })
    }

    async fn run_stream(&self, input: AgentInput) -> Result<AgentStream, HarnessError> {
        let request = self.build_request(&input.message);
        let (tx, rx) = tokio::sync::mpsc::channel(64);

        let llm = self.llm.clone();
        let timeout_ms = self.timeout_ms;

        tokio::spawn(async move {
            let stream_result = tokio::time::timeout(
                Duration::from_millis(timeout_ms),
                llm.generate_content(request, true),
            )
            .await;

            match stream_result {
                Ok(Ok(mut stream)) => {
                    while let Some(chunk) = stream.next().await {
                        if let Ok(resp) = chunk {
                            if let Some(content) = &resp.content {
                                for part in &content.parts {
                                    if let Part::Text { text } = part {
                                        if tx.send(text.clone()).await.is_err() {
                                            return;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(Err(e)) => {
                    let _ = tx.send(format!("[Error: {e}]")).await;
                }
                Err(_) => {
                    let _ = tx.send("[Error: timeout]".to_string()).await;
                }
            }
        });

        Ok(rx)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEST HARNESS: NO API CALLS, DETERMINISTIC
// ═══════════════════════════════════════════════════════════════════════════════

/// A deterministic harness for unit tests — no API calls, no keys needed.
pub struct TestHarness {
    canned_response: String,
}

impl TestHarness {
    pub fn with_response(response: impl Into<String>) -> Self {
        Self {
            canned_response: response.into(),
        }
    }
}

#[async_trait]
impl Harness for TestHarness {
    async fn run(&self, input: AgentInput) -> Result<AgentOutput, HarnessError> {
        Ok(AgentOutput {
            session_id: input.session_id,
            response: self.canned_response.clone(),
            tokens_used: 0,
        })
    }

    async fn run_stream(&self, _input: AgentInput) -> Result<AgentStream, HarnessError> {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let response = self.canned_response.clone();
        tokio::spawn(async move {
            let _ = tx.send(response).await;
        });
        Ok(rx)
    }

    fn name(&self) -> &str {
        "test-harness"
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// AGENT: DEPENDS ONLY ON Arc<dyn Harness>
// ═══════════════════════════════════════════════════════════════════════════════

/// An agent that uses the `Harness` abstraction.
///
/// Never imports `Runner` or `TestHarness` — works with any implementation.
pub struct MyAgent {
    pub harness: Arc<dyn Harness>,
}

impl MyAgent {
    pub fn new(harness: Arc<dyn Harness>) -> Self {
        Self { harness }
    }

    pub async fn handle(&self, msg: &str) -> String {
        let input = AgentInput {
            session_id: uuid::Uuid::new_v4().to_string(),
            message: msg.to_string(),
            context: HashMap::new(),
        };

        match self.harness.run(input).await {
            Ok(out) => out.response,
            Err(e) => format!("Error: {e}"),
        }
    }

    pub async fn handle_stream(&self, msg: &str) {
        let input = AgentInput {
            session_id: uuid::Uuid::new_v4().to_string(),
            message: msg.to_string(),
            context: HashMap::new(),
        };

        match self.harness.run_stream(input).await {
            Ok(mut rx) => {
                while let Some(chunk) = rx.recv().await {
                    print!("{chunk}");
                }
                println!();
            }
            Err(e) => println!("Error: {e}"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// MAIN
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  Harness Pattern — Real LLM + Test Double                   ║");
    println!("║                                                            ║");
    println!("║  Harness (trait) → Runner (Gemini) / TestHarness (mock)     ║");
    println!("║  Agent depends on Arc<dyn Harness> — never the concrete    ║");
    println!("╚══════════════════════════════════════════════════════════════╝\n");

    // ─── Production: Runner with real Gemini ─────────────────────────────────
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  Production: Runner + Gemini (real LLM)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let api_key = std::env::var("GOOGLE_API_KEY")
        .expect("GOOGLE_API_KEY must be set");

    let gemini = Arc::new(
        GeminiModel::new(&api_key, "gemini-2.5-flash")?
    ) as Arc<dyn Llm>;

    let runner = Runner::new("zavora-agent", gemini.clone())
        .with_timeout(15_000)
        .with_system_instruction(
            "You are a concise, helpful assistant. Answer in 1-2 sentences maximum."
        );

    let agent = MyAgent::new(Arc::new(runner));

    // Turn 1: Simple question
    println!("  👤 What is the capital of Kenya?");
    let r1 = agent.handle("What is the capital of Kenya?").await;
    println!("  🤖 {r1}\n");

    // Turn 2: Streaming response
    println!("  👤 Explain quantum computing in one sentence. (streaming)");
    print!("  🤖 ");
    agent.handle_stream("Explain quantum computing in one sentence.").await;
    println!();

    // Turn 3: Creative
    println!("  👤 Write a haiku about Rust programming.");
    let r3 = agent.handle("Write a haiku about Rust programming.").await;
    println!("  🤖 {r3}\n");

    // ─── Test: TestHarness (no API calls) ────────────────────────────────────
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  Test: TestHarness (no API calls)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let test_agent = MyAgent::new(Arc::new(
        TestHarness::with_response("Nairobi is the capital of Kenya."),
    ));

    println!("  👤 What is the capital of Kenya?");
    let r4 = test_agent.handle("What is the capital of Kenya?").await;
    println!("  🤖 {r4}  ← (canned, deterministic, no API call)\n");

    println!("  👤 Completely different question");
    let r5 = test_agent.handle("Completely different question").await;
    println!("  🤖 {r5}  ← (same response — that's the point)\n");

    // ─── Timeout demonstration ───────────────────────────────────────────────
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  Error: Timeout (1ms — impossible deadline)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let timeout_runner = Runner::new("timeout-demo", gemini)
        .with_timeout(1); // 1ms — will always timeout

    let timeout_agent = MyAgent::new(Arc::new(timeout_runner));
    println!("  👤 This will timeout");
    let r6 = timeout_agent.handle("Tell me a joke").await;
    println!("  🤖 {r6}\n");

    // ─── Summary ─────────────────────────────────────────────────────────────
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  Design");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    println!("  Harness (trait)         │ run() / run_stream() / name()");
    println!("    ├── Runner            │ Real LLM (Gemini, OpenAI, etc.)");
    println!("    ├── TestHarness       │ Canned responses, no network");
    println!("    └── (your own impl)   │ Distributed, sandboxed, etc.");
    println!();
    println!("  MyAgent holds Arc<dyn Harness> — swappable at construction.");
    println!("  Same agent code runs in production and tests.");

    Ok(())
}
