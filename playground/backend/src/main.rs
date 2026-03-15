use axum::{
    Router,
    extract::Json,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Mutex;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

#[derive(Deserialize)]
struct RunRequest {
    code: String,
    #[serde(default = "default_example")]
    #[allow(dead_code)]
    example_id: String,
}

fn default_example() -> String {
    "custom".into()
}

#[derive(Serialize)]
struct RunResponse {
    success: bool,
    stdout: String,
    stderr: String,
    duration_ms: u64,
}

#[derive(Serialize, Clone)]
struct Example {
    id: String,
    name: String,
    category: String,
    description: String,
    code: String,
}

async fn health() -> &'static str {
    "ok"
}

async fn list_examples() -> Json<Vec<Example>> {
    Json(get_examples())
}

async fn run_code(
    state: axum::extract::State<AppState>,
    Json(req): Json<RunRequest>,
) -> impl IntoResponse {
    let workspace = &state.workspace_dir;
    // Serialize builds — only one at a time to avoid cargo lock conflicts
    let _lock = state.build_lock.lock().await;

    // Ensure persistent workspace exists with Cargo.toml
    if !workspace.join("Cargo.toml").exists() {
        if let Err(e) = tokio::fs::create_dir_all(workspace.join("src")).await {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(RunResponse {
                success: false,
                stdout: String::new(),
                stderr: format!("Failed to create workspace: {}", e),
                duration_ms: 0,
            }));
        }

        let base_dir = env!("CARGO_MANIFEST_DIR").replace("playground/backend", "")
            .trim_end_matches('/')
            .to_string();
        let adk_rust_base = format!("{}/../adk-rust", base_dir);
        let adk_ui_path = format!("{}/../adk-ui", base_dir);

        let cargo_toml = format!(r#"[package]
name = "playground-run"
version = "0.1.0"
edition = "2024"
rust-version = "1.85.0"

[dependencies]
adk-rust = {{ path = "{adk_rust}/adk-rust", default-features = false, features = ["full"] }}
adk-ui = {{ path = "{adk_ui}" }}
tokio = {{ version = "1", features = ["full"] }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
anyhow = "1"
dotenvy = "0.15"

[patch."https://github.com/zavora-ai/adk-rust"]
adk-core = {{ path = "{adk_rust}/adk-core" }}
adk-agent = {{ path = "{adk_rust}/adk-agent" }}
adk-model = {{ path = "{adk_rust}/adk-model" }}
adk-tool = {{ path = "{adk_rust}/adk-tool" }}
adk-runner = {{ path = "{adk_rust}/adk-runner" }}
adk-server = {{ path = "{adk_rust}/adk-server" }}
adk-session = {{ path = "{adk_rust}/adk-session" }}
adk-artifact = {{ path = "{adk_rust}/adk-artifact" }}
adk-memory = {{ path = "{adk_rust}/adk-memory" }}
adk-cli = {{ path = "{adk_rust}/adk-cli" }}
adk-realtime = {{ path = "{adk_rust}/adk-realtime" }}
adk-graph = {{ path = "{adk_rust}/adk-graph" }}
adk-browser = {{ path = "{adk_rust}/adk-browser" }}
adk-eval = {{ path = "{adk_rust}/adk-eval" }}
adk-ui = {{ path = "{adk_ui}" }}
adk-telemetry = {{ path = "{adk_rust}/adk-telemetry" }}
adk-guardrail = {{ path = "{adk_rust}/adk-guardrail" }}
adk-auth = {{ path = "{adk_rust}/adk-auth" }}
adk-plugin = {{ path = "{adk_rust}/adk-plugin" }}
adk-skill = {{ path = "{adk_rust}/adk-skill" }}
adk-gemini = {{ path = "{adk_rust}/adk-gemini" }}
adk-code = {{ path = "{adk_rust}/adk-code" }}
adk-sandbox = {{ path = "{adk_rust}/adk-sandbox" }}
adk-doc-audit = {{ path = "{adk_rust}/adk-doc-audit" }}
adk-rag = {{ path = "{adk_rust}/adk-rag" }}
adk-audio = {{ path = "{adk_rust}/adk-audio" }}
adk-deploy = {{ path = "{adk_rust}/adk-deploy" }}
adk-rust = {{ path = "{adk_rust}/adk-rust" }}
"#,
            adk_rust = adk_rust_base,
            adk_ui = adk_ui_path,
        );

        if let Err(e) = tokio::fs::write(workspace.join("Cargo.toml"), &cargo_toml).await {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(RunResponse {
                success: false,
                stdout: String::new(),
                stderr: format!("Failed to write Cargo.toml: {}", e),
                duration_ms: 0,
            }));
        }
    }

    // Write the user's code (only thing that changes between runs)
    if let Err(e) = tokio::fs::write(workspace.join("src/main.rs"), &req.code).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(RunResponse {
            success: false,
            stdout: String::new(),
            stderr: format!("Failed to write source: {}", e),
            duration_ms: 0,
        }));
    }

    // Write .env if GOOGLE_API_KEY is set
    if let Ok(key) = std::env::var("GOOGLE_API_KEY") {
        let _ = tokio::fs::write(workspace.join(".env"), format!("GOOGLE_API_KEY={}\n", key)).await;
    }

    let start = std::time::Instant::now();

    // Build (5 min timeout for first build, subsequent builds are fast due to cached deps)
    let build_output = tokio::time::timeout(
        std::time::Duration::from_secs(300),
        Command::new("cargo")
            .arg("build")
            .current_dir(workspace)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
    ).await;

    let build_result = match build_output {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(RunResponse {
                success: false,
                stdout: String::new(),
                stderr: format!("Failed to run cargo: {}", e),
                duration_ms: start.elapsed().as_millis() as u64,
            }));
        }
        Err(_) => {
            return (StatusCode::OK, Json(RunResponse {
                success: false,
                stdout: String::new(),
                stderr: "Build timed out (5 min limit). First builds take longer.".into(),
                duration_ms: start.elapsed().as_millis() as u64,
            }));
        }
    };

    if !build_result.status.success() {
        let stderr = String::from_utf8_lossy(&build_result.stderr).to_string();
        return (StatusCode::OK, Json(RunResponse {
            success: false,
            stdout: String::new(),
            stderr: format!("Compilation failed:\n{}", stderr),
            duration_ms: start.elapsed().as_millis() as u64,
        }));
    }

    // Run with 30s timeout
    let mut run_cmd = Command::new("cargo");
    run_cmd.arg("run").current_dir(workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Forward API keys to child process
    for key in ["GOOGLE_API_KEY", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"] {
        if let Ok(val) = std::env::var(key) {
            run_cmd.env(key, val);
        }
    }

    let run_output = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        run_cmd.output()
    ).await;

    let duration_ms = start.elapsed().as_millis() as u64;

    match run_output {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let raw_stderr = String::from_utf8_lossy(&output.stderr).to_string();
            // Filter out cargo build noise from stderr on successful runs
            let stderr = if output.status.success() {
                raw_stderr.lines()
                    .filter(|line| {
                        let trimmed = line.trim();
                        !trimmed.starts_with("Compiling ")
                            && !trimmed.starts_with("Finished ")
                            && !trimmed.starts_with("Running ")
                            && !trimmed.starts_with("Downloading ")
                            && !trimmed.starts_with("Downloaded ")
                            && !trimmed.starts_with("Building ")
                            && !trimmed.starts_with("Updating ")
                            && !trimmed.starts_with("Locking ")
                            && !trimmed.starts_with("warning: unused")
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
                    .trim()
                    .to_string()
            } else {
                raw_stderr
            };
            (StatusCode::OK, Json(RunResponse {
                success: output.status.success(),
                stdout,
                stderr,
                duration_ms,
            }))
        }
        Ok(Err(e)) => (StatusCode::OK, Json(RunResponse {
            success: false,
            stdout: String::new(),
            stderr: format!("Run failed: {}", e),
            duration_ms,
        })),
        Err(_) => (StatusCode::OK, Json(RunResponse {
            success: false,
            stdout: String::new(),
            stderr: "Execution timed out (30s limit)".into(),
            duration_ms,
        })),
    }
}

fn get_examples() -> Vec<Example> {
    vec![
        Example {
            id: "quickstart".into(),
            name: "Quickstart".into(),
            category: "Getting Started".into(),
            description: "Basic ADK agent with Gemini".into(),
            code: r#"use adk_rust::prelude::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")
        .unwrap_or_else(|_| "demo-key".into());
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-2.5-flash")?);

    let agent = LlmAgentBuilder::new("assistant")
        .instruction("You are a helpful assistant. Be concise.")
        .model(model)
        .build()?;

    println!("Agent '{}' created successfully!", agent.name());
    println!("Ready to process requests.");
    Ok(())
}"#.into(),
        },
        Example {
            id: "multi_tool".into(),
            name: "Agent with Tools".into(),
            category: "Agents".into(),
            description: "LLM agent with custom function tools".into(),
            code: r#"use adk_rust::prelude::*;
use std::sync::Arc;

fn weather_tool() -> FunctionTool {
    FunctionTool::new(
        "get_weather",
        "Get current weather for a city",
        |args: serde_json::Value| {
            Box::pin(async move {
                let city = args["city"].as_str().unwrap_or("unknown");
                Ok(serde_json::json!({
                    "city": city,
                    "temp": "22°C",
                    "condition": "Sunny"
                }))
            })
        },
    )
    .with_parameter("city", "string", "City name", true)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")
        .unwrap_or_else(|_| "demo-key".into());
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-2.5-flash")?);

    let agent = LlmAgentBuilder::new("weather_agent")
        .instruction("You help users check the weather.")
        .model(model)
        .tool(weather_tool())
        .build()?;

    println!("Agent '{}' with {} tool(s) ready!", agent.name(), 1);
    Ok(())
}"#.into(),
        },
        Example {
            id: "sequential_workflow".into(),
            name: "Sequential Pipeline".into(),
            category: "Workflows".into(),
            description: "Chain agents in a sequential pipeline".into(),
            code: r#"use adk_rust::prelude::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")
        .unwrap_or_else(|_| "demo-key".into());
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-2.5-flash")?);

    let researcher: Arc<dyn Agent> = Arc::new(
        LlmAgentBuilder::new("researcher")
            .instruction("Research the topic thoroughly.")
            .model(model.clone())
            .build()?
    );

    let writer: Arc<dyn Agent> = Arc::new(
        LlmAgentBuilder::new("writer")
            .instruction("Write a clear summary from the research.")
            .model(model.clone())
            .build()?
    );

    let pipeline = SequentialAgent::new(
        "research_pipeline",
        vec![researcher, writer],
    );

    println!("Pipeline '{}' ready with 2 stages", pipeline.name());
    Ok(())
}"#.into(),
        },
        Example {
            id: "graph_agent".into(),
            name: "Graph Agent".into(),
            category: "Graph".into(),
            description: "Stateful graph-based agent with conditional routing".into(),
            code: r#"use adk_rust::prelude::*;
use adk_rust::graph::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")
        .unwrap_or_else(|_| "demo-key".into());
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-2.5-flash")?);

    let graph = GraphBuilder::new("classifier")
        .add_llm_node("analyze", model.clone(),
            "Classify the input as 'positive', 'negative', or 'neutral'.")
        .add_llm_node("respond_positive", model.clone(),
            "Generate an enthusiastic response.")
        .add_llm_node("respond_negative", model.clone(),
            "Generate an empathetic response.")
        .add_llm_node("respond_neutral", model,
            "Generate a balanced response.")
        .set_entry("analyze")
        .add_conditional_edge("analyze", |state| {
            let text = state.get_last_text().unwrap_or_default().to_lowercase();
            if text.contains("positive") { "respond_positive".into() }
            else if text.contains("negative") { "respond_negative".into() }
            else { "respond_neutral".into() }
        })
        .build()?;

    println!("Graph '{}' built with conditional routing!", graph.name());
    Ok(())
}"#.into(),
        },
        Example {
            id: "structured_output".into(),
            name: "Structured Output".into(),
            category: "Agents".into(),
            description: "Get typed JSON responses from an agent".into(),
            code: r#"use adk_rust::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
struct MovieReview {
    title: String,
    rating: f32,
    summary: String,
    recommended: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")
        .unwrap_or_else(|_| "demo-key".into());
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-2.5-flash")?);

    let agent = LlmAgentBuilder::new("reviewer")
        .instruction("You review movies. Always respond with structured JSON.")
        .model(model)
        .output_schema::<MovieReview>()
        .build()?;

    println!("Structured output agent '{}' ready!", agent.name());
    println!("Expected schema: MovieReview {{ title, rating, summary, recommended }}");
    Ok(())
}"#.into(),
        },
        Example {
            id: "parallel_workflow".into(),
            name: "Parallel Analysis".into(),
            category: "Workflows".into(),
            description: "Run multiple agents in parallel and merge results".into(),
            code: r#"use adk_rust::prelude::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")
        .unwrap_or_else(|_| "demo-key".into());
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-2.5-flash")?);

    let sentiment: Arc<dyn Agent> = Arc::new(
        LlmAgentBuilder::new("sentiment")
            .instruction("Analyze the sentiment of the text.")
            .model(model.clone())
            .build()?
    );

    let keywords: Arc<dyn Agent> = Arc::new(
        LlmAgentBuilder::new("keywords")
            .instruction("Extract key topics from the text.")
            .model(model.clone())
            .build()?
    );

    let summary: Arc<dyn Agent> = Arc::new(
        LlmAgentBuilder::new("summary")
            .instruction("Write a one-line summary.")
            .model(model)
            .build()?
    );

    let parallel = ParallelAgent::new(
        "text_analyzer",
        vec![sentiment, keywords, summary],
    );

    println!("Parallel analyzer '{}' ready with 3 branches", parallel.name());
    Ok(())
}"#.into(),
        },
    ]
}

#[derive(Clone)]
struct AppState {
    workspace_dir: PathBuf,
    build_lock: Arc<Mutex<()>>,
}

#[tokio::main]
async fn main() {
    let workspace_dir = std::env::temp_dir().join("adk-playground-workspace");
    let state = AppState {
        workspace_dir,
        build_lock: Arc::new(Mutex::new(())),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/examples", get(list_examples))
        .route("/api/run", post(run_code))
        .fallback_service(ServeDir::new("../frontend/dist"))
        .layer(cors)
        .with_state(state);

    let port = 9876;
    let addr = format!("0.0.0.0:{}", port);
    println!("🚀 ADK Playground server running on http://localhost:{}", port);

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
