//! Artifact-Powered Agent — versioned file storage during conversations
//!
//! Agent that generates content and saves it as versioned artifacts,
//! then retrieves and compares versions on request.

use adk_rust::prelude::*;
use adk_rust::session::{SessionService, CreateRequest};
use adk_rust::futures::StreamExt;
use adk_core::{Part, UserId, SessionId};
use adk_artifact::{ArtifactService, InMemoryArtifactService, SaveRequest, LoadRequest, ListRequest};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

static ARTIFACT_SVC: OnceLock<Arc<InMemoryArtifactService>> = OnceLock::new();

#[derive(Deserialize, JsonSchema)]
struct SaveArgs {
    /// File name to save (e.g. "report.md", "summary.txt")
    file_name: String,
    /// Content to save
    content: String,
}

/// Save content as a versioned artifact. Returns the version number.
#[tool]
async fn save_artifact(args: SaveArgs) -> adk_tool::Result<serde_json::Value> {
    let svc = ARTIFACT_SVC.get().unwrap();
    let resp = match svc.save(SaveRequest {
        app_name: "playground".into(),
        user_id: "user".into(),
        session_id: "s1".into(),
        file_name: args.file_name.clone(),
        part: Part::Text { text: args.content },
        version: None,
    }).await {
        Ok(r) => r,
        Err(e) => return Ok(serde_json::json!({ "error": e.to_string() })),
    };

    Ok(serde_json::json!({
        "saved": args.file_name,
        "version": resp.version,
    }))
}

#[derive(Deserialize, JsonSchema)]
struct LoadArgs {
    /// File name to load
    file_name: String,
    /// Optional version number (omit for latest)
    version: Option<i64>,
}

/// Load an artifact by name. Returns the content and version.
#[tool]
async fn load_artifact(args: LoadArgs) -> adk_tool::Result<serde_json::Value> {
    let svc = ARTIFACT_SVC.get().unwrap();
    match svc.load(LoadRequest {
        app_name: "playground".into(),
        user_id: "user".into(),
        session_id: "s1".into(),
        file_name: args.file_name.clone(),
        version: args.version,
    }).await {
        Ok(resp) => {
            let text = match &resp.part {
                Part::Text { text } => text.clone(),
                _ => "(binary data)".into(),
            };
            Ok(serde_json::json!({
                "file": args.file_name,
                "content": text,
            }))
        }
        Err(e) => Ok(serde_json::json!({ "error": e.to_string() })),
    }
}

/// List all saved artifacts in the current session.
#[tool]
async fn list_artifacts() -> adk_tool::Result<serde_json::Value> {
    let svc = ARTIFACT_SVC.get().unwrap();
    match svc.list(ListRequest {
        app_name: "playground".into(),
        user_id: "user".into(),
        session_id: "s1".into(),
    }).await {
        Ok(resp) => Ok(serde_json::json!({ "files": resp.file_names })),
        Err(e) => Ok(serde_json::json!({ "error": e.to_string() })),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== Artifact-Powered Agent ===\n");

    let artifact_svc = Arc::new(InMemoryArtifactService::new());
    let _ = ARTIFACT_SVC.set(artifact_svc.clone());

    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("artifact_agent")
            .instruction(
                "You are a writing assistant with artifact storage.\n\
                 You can save content as versioned files, load previous versions, and list files.\n\
                 When asked to write something, save it as an artifact.\n\
                 When asked to revise, load the previous version, improve it, and save a new version.\n\
                 Always confirm what you saved and the version number."
            )
            .model(model)
            .tool(Arc::new(SaveArtifact))
            .tool(Arc::new(LoadArtifact))
            .tool(Arc::new(ListArtifacts))
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
        artifact_service: Some(artifact_svc),
        memory_service: None,
        plugin_manager: None,
        run_config: None,
        compaction_config: None,
        context_cache_config: None,
        cache_capable: None,
        request_context: None,
        cancellation_token: None,
    })?;

    let query = "Write a short haiku about Rust programming and save it as poem.txt";
    println!("**User:** {}\n", query);
    print!("**Agent:** ");

    let message = Content::new("user").with_text(query);
    let mut stream = runner.run(UserId::new("user")?, SessionId::new("s1")?, message).await?;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() { print!("{}", text); }
            }
        }
    }
    println!();

    Ok(())
}
