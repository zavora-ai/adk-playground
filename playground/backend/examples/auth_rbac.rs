//! RBAC Agent — role-based access control on tools
//!
//! Demonstrates adk-auth's AccessControl system: define roles with
//! allow/deny permissions, assign users to roles, and gate tool access.
//! The agent's tools check permissions before executing.

use adk_auth::{AccessControl, Permission, Role};
use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

static ACCESS_CONTROL: OnceLock<Arc<AccessControl>> = OnceLock::new();
static CURRENT_USER: &str = "analyst@company.com";

#[derive(Deserialize, JsonSchema)]
struct SearchArgs {
    /// Search query
    query: String,
}

/// Search the company database. Requires 'search' tool permission.
#[tool]
async fn search_database(args: SearchArgs) -> adk_tool::Result<serde_json::Value> {
    let ac = ACCESS_CONTROL.get().unwrap();
    if let Err(e) = ac.check(CURRENT_USER, &Permission::Tool("search_database".into())) {
        return Ok(serde_json::json!({ "error": format!("ACCESS DENIED: {}", e) }));
    }
    Ok(serde_json::json!({
        "query": args.query,
        "results": [
            {"id": 1, "title": "Q1 Revenue Report", "summary": "Revenue grew 15% YoY"},
            {"id": 2, "title": "Q2 Forecast", "summary": "Projected 12% growth"},
        ]
    }))
}

#[derive(Deserialize, JsonSchema)]
struct DeleteArgs {
    /// Record ID to delete
    record_id: u32,
}

/// Delete a record from the database. Requires 'admin_delete' tool permission.
#[tool]
async fn admin_delete(args: DeleteArgs) -> adk_tool::Result<serde_json::Value> {
    let ac = ACCESS_CONTROL.get().unwrap();
    if let Err(e) = ac.check(CURRENT_USER, &Permission::Tool("admin_delete".into())) {
        return Ok(serde_json::json!({ "error": format!("ACCESS DENIED: {}", e) }));
    }
    Ok(serde_json::json!({
        "deleted": args.record_id,
        "status": "success",
    }))
}

#[derive(Deserialize, JsonSchema)]
struct SummarizeArgs {
    /// Text to summarize
    text: String,
}

/// Summarize text. Requires 'summarize' tool permission.
#[tool]
async fn summarize(args: SummarizeArgs) -> adk_tool::Result<serde_json::Value> {
    let ac = ACCESS_CONTROL.get().unwrap();
    if let Err(e) = ac.check(CURRENT_USER, &Permission::Tool("summarize".into())) {
        return Ok(serde_json::json!({ "error": format!("ACCESS DENIED: {}", e) }));
    }
    Ok(serde_json::json!({
        "summary": format!("Summary of {} chars of text", args.text.len()),
        "status": "success",
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== RBAC Agent — Role-Based Access Control ===\n");

    // ── 1. Define roles ──
    let admin = Role::new("admin")
        .allow(Permission::AllTools)
        .allow(Permission::AllAgents);

    let analyst = Role::new("analyst")
        .allow(Permission::Tool("search_database".into()))
        .allow(Permission::Tool("summarize".into()))
        .deny(Permission::Tool("admin_delete".into()));

    let viewer = Role::new("viewer")
        .allow(Permission::Tool("search_database".into()))
        .deny(Permission::Tool("admin_delete".into()))
        .deny(Permission::Tool("summarize".into()));

    println!("Roles defined:");
    println!("  admin   → AllTools (full access)");
    println!("  analyst → search_database + summarize (no delete)");
    println!("  viewer  → search_database only\n");

    // ── 2. Assign users to roles ──
    let ac = AccessControl::builder()
        .role(admin)
        .role(analyst)
        .role(viewer)
        .assign("admin@company.com", "admin")
        .assign("analyst@company.com", "analyst")
        .assign("viewer@company.com", "viewer")
        .build()?;

    // ── 3. Demo permission checks ──
    println!("Permission checks for analyst@company.com:");
    let checks = [
        (
            "search_database",
            ac.check(
                "analyst@company.com",
                &Permission::Tool("search_database".into()),
            ),
        ),
        (
            "summarize",
            ac.check("analyst@company.com", &Permission::Tool("summarize".into())),
        ),
        (
            "admin_delete",
            ac.check(
                "analyst@company.com",
                &Permission::Tool("admin_delete".into()),
            ),
        ),
    ];
    for (tool_name, result) in &checks {
        let status = if result.is_ok() {
            "✓ ALLOWED"
        } else {
            "✗ DENIED"
        };
        println!("  {} → {}", tool_name, status);
    }
    println!();

    let ac = Arc::new(ac);
    let _ = ACCESS_CONTROL.set(ac.clone());

    // ── 4. Build agent with RBAC-gated tools ──
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("rbac_agent")
            .instruction(
                "You are a data assistant. You have three tools:\n\
                 - search_database: search company records\n\
                 - summarize: summarize text\n\
                 - admin_delete: delete records (admin only)\n\n\
                 The current user is an analyst with limited permissions.\n\
                 If a tool returns ACCESS DENIED, explain that the user lacks permission.\n\
                 Try to fulfill the request using the tools you have access to.",
            )
            .model(model)
            .tool(Arc::new(SearchDatabase))
            .tool(Arc::new(AdminDelete))
            .tool(Arc::new(Summarize))
            .build()?,
    );

    let sessions = Arc::new(InMemorySessionService::new());
    sessions
        .create(CreateRequest {
            app_name: "playground".into(),
            user_id: "user".into(),
            session_id: Some("s1".into()),
            state: HashMap::new(),
        })
        .await?;

    let runner = Runner::new(RunnerConfig {
        app_name: "playground".into(),
        agent,
        session_service: sessions,
        artifact_service: None,
        memory_service: None,
        plugin_manager: None,
        run_config: None,
        compaction_config: None,
        context_cache_config: None,
        cache_capable: None,
        request_context: None,
        cancellation_token: None,
    })?;

    let query = "Search for revenue reports, summarize the results, then delete record #1.";
    println!("**User:** {}\n", query);
    print!("**Agent:** ");

    let message = Content::new("user").with_text(query);
    let mut stream = runner
        .run(UserId::new("user")?, SessionId::new("s1")?, message)
        .await?;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() {
                    print!("{}", text);
                }
            }
        }
    }
    println!();

    Ok(())
}
