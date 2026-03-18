mod examples;

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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    traces: Vec<TraceEvent>,
}

#[derive(Serialize, Clone)]
struct TraceEvent {
    timestamp_ms: u64,
    level: String,
    name: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    kind: String, // "agent", "llm", "tool_call", "tool_result", "info", "warn"
    #[serde(skip_serializing_if = "String::is_empty")]
    target: String,
}

/// Returns true if the server is in public (restricted) mode.
/// In public mode, only registered examples can be executed.
fn is_public_mode() -> bool {
    std::env::var("PLAYGROUND_MODE")
        .map(|v| v.eq_ignore_ascii_case("public"))
        .unwrap_or(false)
}

async fn health() -> &'static str {
    "ok"
}

async fn list_examples() -> Json<Vec<examples::Example>> {
    Json(examples::load_examples())
}

/// Resolve the code to execute. In public mode, only registered examples are allowed.
/// Returns Ok(code) or Err(response) if rejected.
fn resolve_code(req: &RunRequest) -> Result<String, RunResponse> {
    if is_public_mode() {
        // Public mode: only allow registered examples
        let examples = examples::load_examples();
        if let Some(ex) = examples.iter().find(|e| e.id == req.example_id) {
            Ok(ex.code.clone())
        } else {
            Err(RunResponse {
                success: false,
                stdout: String::new(),
                stderr: "Public mode: only registered examples can be executed. \
                         Select an example from the sidebar."
                    .into(),
                duration_ms: 0,
                traces: Vec::new(),
            })
        }
    } else {
        // Local mode: run whatever code the user sends
        Ok(req.code.clone())
    }
}

async fn run_code(
    state: axum::extract::State<AppState>,
    Json(req): Json<RunRequest>,
) -> impl IntoResponse {
    // Resolve code (enforces public mode restrictions)
    let code = match resolve_code(&req) {
        Ok(c) => c,
        Err(resp) => return (StatusCode::OK, Json(resp)),
    };

    let workspace = &state.workspace_dir;
    let _lock = state.build_lock.lock().await;

    // Create workspace with Cargo.toml on first run
    if !workspace.join("Cargo.toml").exists() {
        if let Err(e) = tokio::fs::create_dir_all(workspace.join("src")).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(RunResponse {
                    success: false,
                    stdout: String::new(),
                    stderr: format!("Failed to create workspace: {}", e),
                    duration_ms: 0,
                    traces: Vec::new(),
                }),
            );
        }

        let cargo_toml = r#"[package]
name = "playground-run"
version = "0.1.0"
edition = "2021"

[dependencies]
adk-rust = { version = "0.4.0", default-features = false, features = ["full"] }
adk-tool = "0.4.0"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
schemars = "0.8"
async-trait = "0.1"
anyhow = "1"
dotenvy = "0.15"
tracing-subscriber = { version = "0.3", features = ["json", "env-filter"] }
"#;

        if let Err(e) = tokio::fs::write(workspace.join("Cargo.toml"), cargo_toml).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(RunResponse {
                    success: false,
                    stdout: String::new(),
                    stderr: format!("Failed to write Cargo.toml: {}", e),
                    duration_ms: 0,
                    traces: Vec::new(),
                }),
            );
        }
    }

    // Inject tracing subscriber init into the code
    let code_with_tracing = inject_tracing_init(&code);

    if let Err(e) = tokio::fs::write(workspace.join("src/main.rs"), &code_with_tracing).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(RunResponse {
                success: false,
                stdout: String::new(),
                stderr: format!("Failed to write source: {}", e),
                duration_ms: 0,
                traces: Vec::new(),
            }),
        );
    }

    // Forward API keys
    let mut env_lines = Vec::new();
    for key in ["GOOGLE_API_KEY", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"] {
        if let Ok(val) = std::env::var(key) {
            env_lines.push(format!("{}={}", key, val));
        }
    }
    if !env_lines.is_empty() {
        let _ = tokio::fs::write(workspace.join(".env"), env_lines.join("\n")).await;
    }

    let start = std::time::Instant::now();

    // Build (5 min timeout for first build)
    let build_output = tokio::time::timeout(
        std::time::Duration::from_secs(300),
        Command::new("cargo")
            .arg("build")
            .current_dir(workspace)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await;

    let build_result = match build_output {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(RunResponse {
                    success: false,
                    stdout: String::new(),
                    stderr: format!("Failed to run cargo: {}", e),
                    duration_ms: start.elapsed().as_millis() as u64,
                    traces: Vec::new(),
                }),
            )
        }
        Err(_) => {
            return (
                StatusCode::OK,
                Json(RunResponse {
                    success: false,
                    stdout: String::new(),
                    stderr: "Build timed out (5 min limit). First builds take longer.".into(),
                    duration_ms: start.elapsed().as_millis() as u64,
                    traces: Vec::new(),
                }),
            )
        }
    };

    if !build_result.status.success() {
        let stderr = String::from_utf8_lossy(&build_result.stderr).to_string();
        return (
            StatusCode::OK,
            Json(RunResponse {
                success: false,
                stdout: String::new(),
                stderr: format!("Compilation failed:\n{}", stderr),
                duration_ms: start.elapsed().as_millis() as u64,
                traces: Vec::new(),
            }),
        );
    }

    // Run with 30s timeout
    let mut run_cmd = Command::new("cargo");
    run_cmd
        .arg("run")
        .current_dir(workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("RUST_LOG", "info,hyper=warn,reqwest=warn,h2=warn,rustls=warn,tonic=warn");

    for key in ["GOOGLE_API_KEY", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"] {
        if let Ok(val) = std::env::var(key) {
            run_cmd.env(key, val);
        }
    }

    let run_output = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        run_cmd.output(),
    )
    .await;

    let duration_ms = start.elapsed().as_millis() as u64;

    match run_output {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let raw_stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let (stderr, traces) = if output.status.success() {
                parse_traces(&raw_stderr, 0)
            } else {
                (raw_stderr, Vec::new())
            };
            (
                StatusCode::OK,
                Json(RunResponse {
                    success: output.status.success(),
                    stdout,
                    stderr,
                    duration_ms,
                    traces,
                }),
            )
        }
        Ok(Err(e)) => (
            StatusCode::OK,
            Json(RunResponse {
                success: false,
                stdout: String::new(),
                stderr: format!("Run failed: {}", e),
                duration_ms,
                traces: Vec::new(),
            }),
        ),
        Err(_) => (
            StatusCode::OK,
            Json(RunResponse {
                success: false,
                stdout: String::new(),
                stderr: "Execution timed out (30s limit)".into(),
                duration_ms,
                traces: Vec::new(),
            }),
        ),
    }
}


/// Inject a JSON tracing subscriber init into user code so we capture structured traces.
fn inject_tracing_init(code: &str) -> String {
    // Insert tracing init right after `dotenvy::dotenv().ok();`
    let tracing_init = r#"
    // --- Playground tracing (auto-injected) ---
    tracing_subscriber::fmt()
        .json()
        .with_target(true)
        .with_env_filter(tracing_subscriber::EnvFilter::new("info,hyper=warn,reqwest=warn,h2=warn,rustls=warn,tonic=warn"))
        .with_writer(std::io::stderr)
        .init();
    // --- End playground tracing ---"#;

    if let Some(pos) = code.find("dotenvy::dotenv().ok();") {
        let insert_at = pos + "dotenvy::dotenv().ok();".len();
        let mut result = String::with_capacity(code.len() + tracing_init.len());
        result.push_str(&code[..insert_at]);
        result.push_str(tracing_init);
        result.push_str(&code[insert_at..]);
        result
    } else {
        code.to_string()
    }
}

/// Parse tracing output from stderr into structured trace events.
/// Returns (user_stderr, traces).
fn parse_traces(raw_stderr: &str, _run_start_ms: u64) -> (String, Vec<TraceEvent>) {
    let mut traces = Vec::new();
    let mut user_lines = Vec::new();
    let mut ms_counter: u64 = 0;
    let mut run_start: Option<chrono::DateTime<chrono::Utc>> = None;

    for line in raw_stderr.lines() {
        let t = line.trim();

        // Skip cargo noise
        if t.starts_with("Compiling ")
            || t.starts_with("Finished ")
            || t.starts_with("Running ")
            || t.starts_with("Downloading ")
            || t.starts_with("Downloaded ")
            || t.starts_with("Building ")
            || t.starts_with("Updating ")
            || t.starts_with("Locking ")
            || t.starts_with("warning: unused")
            || t.starts_with("warning: `playground-run`")
            || t.contains("generated 1 warning")
        {
            continue;
        }

        // Try to parse tracing JSON lines (from tracing-subscriber JSON format)
        if t.starts_with('{') {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(t) {
                // Capture first timestamp as run start for relative timing
                if run_start.is_none() {
                    run_start = json.get("timestamp")
                        .and_then(|v| v.as_str())
                        .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
                        .map(|dt| dt.with_timezone(&chrono::Utc));
                }
                if let Some(evt) = parse_trace_json(&json, ms_counter, run_start.as_ref()) {
                    ms_counter = evt.timestamp_ms + 1;
                    traces.push(evt);
                    continue;
                }
            }
        }

        // Parse structured tracing text output: "  INFO agent.execute{...}: message"
        if let Some(evt) = parse_trace_text(t, ms_counter) {
            ms_counter = evt.timestamp_ms + 1;
            traces.push(evt);
            continue;
        }

        // Everything else is user stderr
        user_lines.push(line);
    }

    // If no structured traces found, synthesize from stdout patterns
    let user_stderr = user_lines.join("\n").trim().to_string();
    (user_stderr, traces)
}

fn parse_trace_json(json: &serde_json::Value, ms: u64, run_start: Option<&chrono::DateTime<chrono::Utc>>) -> Option<TraceEvent> {
    // tracing-subscriber JSON format:
    // {"timestamp":"...","level":"INFO","fields":{"message":"...","tool.name":"..."},"target":"adk_agent::llm_agent","span":{"name":"agent.execute"},"spans":[...]}
    let level = json.get("level")?.as_str()?.to_lowercase();
    let target = json.get("target").and_then(|v| v.as_str()).unwrap_or("");
    let fields = json.get("fields").cloned().unwrap_or(serde_json::Value::Null);
    let message = fields.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string();

    // Parse real timestamp from tracing-subscriber output
    let timestamp_ms = json.get("timestamp")
        .and_then(|v| v.as_str())
        .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
        .map(|dt| {
            if let Some(start) = run_start {
                let diff = dt.signed_duration_since(*start);
                diff.num_milliseconds().max(0) as u64
            } else {
                ms
            }
        })
        .unwrap_or(ms);

    // Extract span info from "spans" array or "span" object
    let span_name = json.get("span")
        .and_then(|s| s.get("name"))
        .and_then(|n| n.as_str())
        .or_else(|| {
            json.get("spans")
                .and_then(|s| s.as_array())
                .and_then(|arr| arr.last())
                .and_then(|s| s.get("name"))
                .and_then(|n| n.as_str())
        })
        .unwrap_or("");

    // Extract agent name from spans
    let agent = json.get("spans")
        .and_then(|s| s.as_array())
        .and_then(|arr| {
            arr.iter().find_map(|s| {
                s.get("agent.name").and_then(|v| v.as_str())
                    .or_else(|| s.get("gcp.vertex.agent.agent_name").and_then(|v| v.as_str()))
            })
        })
        .or_else(|| {
            fields.get("agent.name").and_then(|v| v.as_str())
        })
        .map(|s| s.to_string());

    // Extract tool name
    let tool = fields.get("tool.name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Extract detail (tool args or result)
    let detail = fields.get("tool.args")
        .or_else(|| fields.get("tool.result"))
        .and_then(|v| v.as_str())
        .map(|s| {
            if s.len() > 500 { format!("{}...", &s[..500]) } else { s.to_string() }
        });

    let kind = classify_trace(span_name, target, &message, tool.as_deref());

    // Skip noisy internal traces
    if target.starts_with("hyper") || target.starts_with("reqwest") || target.starts_with("h2")
        || target.starts_with("rustls") || target.starts_with("tonic") || target.starts_with("tower")
        || target.starts_with("mio") || target.starts_with("want")
    {
        return None;
    }

    Some(TraceEvent {
        timestamp_ms,
        level,
        name: if span_name.is_empty() { target.to_string() } else { span_name.to_string() },
        message,
        agent,
        tool,
        detail,
        kind,
        target: target.to_string(),
    })
}

fn parse_trace_text(line: &str, ms: u64) -> Option<TraceEvent> {
    // Match patterns like: "  INFO agent.execute{...}: Agent execution complete"
    // or "  INFO tool_call agent.name=foo tool.name=bar"
    let trimmed = line.trim();

    let (level, rest) = if trimmed.starts_with("INFO ") {
        ("info", &trimmed[5..])
    } else if trimmed.starts_with("WARN ") {
        ("warn", &trimmed[5..])
    } else if trimmed.starts_with("DEBUG ") {
        ("debug", &trimmed[6..])
    } else {
        return None;
    };

    let agent = extract_field(rest, "agent.name=");
    let tool = extract_field(rest, "tool.name=");
    let tool_args = extract_field(rest, "tool.args=");
    let tool_result = extract_field(rest, "tool.result=");

    let (name, message) = if let Some(colon_pos) = rest.find(": ") {
        let span_part = &rest[..colon_pos];
        let msg_part = &rest[colon_pos + 2..];
        let name = span_part.split('{').next().unwrap_or(span_part).trim();
        (name.to_string(), msg_part.to_string())
    } else {
        (rest.split_whitespace().next().unwrap_or("").to_string(), rest.to_string())
    };

    let kind = classify_trace(&name, "", &message, tool.as_deref());

    let detail = tool_args.or(tool_result);

    Some(TraceEvent {
        timestamp_ms: ms,
        level: level.to_string(),
        name,
        message,
        agent,
        tool,
        detail,
        kind,
        target: String::new(),
    })
}

fn extract_field(text: &str, prefix: &str) -> Option<String> {
    text.find(prefix).map(|start| {
        let val_start = start + prefix.len();
        let val = &text[val_start..];
        val.split_whitespace().next().unwrap_or("").to_string()
    })
}

fn classify_trace(name: &str, target: &str, message: &str, tool: Option<&str>) -> String {
    let msg_lower = message.to_lowercase();
    if name.contains("agent.execute") || msg_lower.contains("agent execution") || msg_lower.contains("agent '") {
        "agent".to_string()
    } else if name.contains("llm") || msg_lower.contains("llm_response") || msg_lower.contains("llm_call")
        || target.contains("gemini") || target.contains("model") {
        "llm".to_string()
    } else if msg_lower.contains("tool_call") || (tool.is_some() && msg_lower.contains("tool_call")) {
        "tool_call".to_string()
    } else if msg_lower.contains("tool_result") {
        "tool_result".to_string()
    } else if msg_lower.contains("tool_error") || msg_lower.contains("tool_timeout") {
        "tool_error".to_string()
    } else if tool.is_some() {
        "tool_call".to_string()
    } else if msg_lower.contains("warn") || message.contains("WARN") {
        "warn".to_string()
    } else {
        "info".to_string()
    }
}

/// Info endpoint: tells the frontend what mode the server is in
async fn server_info() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "mode": if is_public_mode() { "public" } else { "local" },
        "version": env!("CARGO_PKG_VERSION"),
        "custom_code_enabled": !is_public_mode(),
    }))
}

#[derive(Clone)]
struct AppState {
    workspace_dir: PathBuf,
    build_lock: Arc<Mutex<()>>,
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let mode = if is_public_mode() { "public" } else { "local" };

    let state = AppState {
        workspace_dir: PathBuf::from("/var/tmp/adk-playground-workspace"),
        build_lock: Arc::new(Mutex::new(())),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/info", get(server_info))
        .route("/api/examples", get(list_examples))
        .route("/api/run", post(run_code))
        .fallback_service(ServeDir::new("../frontend/dist"))
        .layer(cors)
        .with_state(state);

    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(9876u16);

    println!("🚀 ADK Playground server running on http://localhost:{}", port);
    println!("   Mode: {} | Custom code: {}", mode, !is_public_mode());

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port))
        .await
        .unwrap();
    axum::serve(listener, app).await.unwrap();
}
