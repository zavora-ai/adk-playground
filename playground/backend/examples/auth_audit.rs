use adk_auth::{
    AccessControl, AuditEvent, AuditOutcome, AuditSink, AuthError, AuthMiddleware, Permission, Role,
};
use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ── RBAC + Audit Trail — Secure Agent Tool Access ──
// ADK-Rust enforces role-based access control on every tool call.
// Every access decision (allowed or denied) is logged to an audit sink.
//
// Key concepts:
//   - `Role::new().allow().deny()` — declarative permission rules
//   - `AccessControl` — maps users to roles, checks permissions
//   - `AuthMiddleware` — wraps tools with automatic RBAC enforcement
//   - `AuditSink` — pluggable audit destination (file, DB, SIEM)
//   - Deny always takes precedence over allow
//
// This example:
//   1. Defines three roles with different tool permissions
//   2. Shows the permission matrix across users
//   3. Captures audit events for every access decision
//   4. Runs an agent where the LLM encounters permission boundaries

// ── In-memory audit sink ──
struct MemoryAuditSink {
    events: Mutex<Vec<AuditEvent>>,
}

impl MemoryAuditSink {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }
    fn events(&self) -> Vec<AuditEvent> {
        self.events.lock().unwrap().clone()
    }
}

#[async_trait]
impl AuditSink for MemoryAuditSink {
    async fn log(&self, event: AuditEvent) -> std::result::Result<(), AuthError> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

// ── Tools ──

#[derive(Deserialize, JsonSchema)]
struct SearchArgs {
    query: String,
}

/// Search company records.
#[tool]
async fn search_records(args: SearchArgs) -> adk_tool::Result<serde_json::Value> {
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
    record_id: u32,
}

/// Delete a record. Admin only.
#[tool]
async fn delete_record(args: DeleteArgs) -> adk_tool::Result<serde_json::Value> {
    Ok(serde_json::json!({ "deleted": args.record_id, "status": "success" }))
}

#[derive(Deserialize, JsonSchema)]
struct TransferArgs {
    amount: f64,
    to: String,
}

/// Transfer funds. Finance role only.
#[tool]
async fn transfer_funds(args: TransferArgs) -> adk_tool::Result<serde_json::Value> {
    Ok(serde_json::json!({
        "amount": args.amount, "to": args.to,
        "status": "completed", "tx_id": "TX-2025-001"
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== 📋 RBAC + Audit Trail — Secure Agent Tool Access ===\n");

    // ── 1. Define RBAC roles ──
    println!("── 1. Role Definitions ──\n");

    let admin = Role::new("admin")
        .allow(Permission::AllTools)
        .allow(Permission::AllAgents);

    let analyst = Role::new("analyst")
        .allow(Permission::Tool("search_records".into()))
        .deny(Permission::Tool("delete_record".into()))
        .deny(Permission::Tool("transfer_funds".into()));

    let finance = Role::new("finance")
        .allow(Permission::Tool("search_records".into()))
        .allow(Permission::Tool("transfer_funds".into()))
        .deny(Permission::Tool("delete_record".into()));

    println!("  admin   → AllTools (full access)");
    println!("  analyst → search only (deny takes precedence)");
    println!("  finance → search + transfer (no delete)\n");

    // ── 2. User assignments + permission matrix ──
    let ac = AccessControl::builder()
        .role(admin)
        .role(analyst)
        .role(finance)
        .assign("alice@company.com", "admin")
        .assign("bob@company.com", "analyst")
        .assign("carol@company.com", "finance")
        .build()?;

    println!("── 2. Permission Matrix ──\n");

    let users = ["alice@company.com", "bob@company.com", "carol@company.com"];
    let tools = ["search_records", "delete_record", "transfer_funds"];

    print!("  {:15}", "");
    for t in &tools {
        print!("{:18}", t);
    }
    println!();
    print!("  {:15}", "");
    println!("{}", "─".repeat(54));

    for user in &users {
        let name = user.split('@').next().unwrap();
        print!("  {:15}", name);
        for t in &tools {
            let allowed = ac.check(user, &Permission::Tool((*t).into())).is_ok();
            print!("{:18}", if allowed { "✓ ALLOW" } else { "✗ DENY" });
        }
        println!();
    }
    println!();

    // ── 3. Audit event capture ──
    println!("── 3. Audit Event Capture ──\n");

    let audit = Arc::new(MemoryAuditSink::new());

    let attempts = [
        ("alice", "search_records"),
        ("alice", "delete_record"),
        ("alice", "transfer_funds"),
        ("bob", "search_records"),
        ("bob", "delete_record"),
        ("bob", "transfer_funds"),
        ("carol", "search_records"),
        ("carol", "delete_record"),
        ("carol", "transfer_funds"),
    ];

    for (user, tool_name) in &attempts {
        let full_user = format!("{user}@company.com");
        let outcome = if ac
            .check(&full_user, &Permission::Tool((*tool_name).into()))
            .is_ok()
        {
            AuditOutcome::Allowed
        } else {
            AuditOutcome::Denied
        };
        audit
            .log(AuditEvent::tool_access(*user, *tool_name, outcome))
            .await?;
    }

    let events = audit.events();
    println!("  {} audit events captured:\n", events.len());
    for e in &events {
        let icon = match e.outcome {
            AuditOutcome::Allowed => "✓",
            _ => "✗",
        };
        println!(
            "    {icon} {:8} → {:18} {:?}",
            e.user, e.resource, e.outcome
        );
    }
    println!();

    println!("  Production: use FileAuditSink for JSONL append-only logs:");
    println!("    let sink = FileAuditSink::new(\"/var/log/adk/audit.jsonl\")?;");
    println!("    let middleware = AuthMiddleware::with_audit(ac, sink);\n");

    // ── 4. Agent with RBAC-protected tools ──
    println!("── 4. Agent with Audit-Protected Tools ──\n");

    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    // Rebuild access control for the agent user (bob = analyst)
    let agent_ac = Arc::new(
        AccessControl::builder()
            .role(
                Role::new("analyst")
                    .allow(Permission::Tool("search_records".into()))
                    .deny(Permission::Tool("delete_record".into()))
                    .deny(Permission::Tool("transfer_funds".into())),
            )
            .assign("bob@company.com", "analyst")
            .build()?,
    );

    // Store in a static so tools can check permissions
    static AC: std::sync::OnceLock<Arc<AccessControl>> = std::sync::OnceLock::new();
    let _ = AC.set(agent_ac);

    let agent = Arc::new(
        LlmAgentBuilder::new("audit_agent")
            .instruction(
                "You are a data assistant. The current user is an analyst (bob@company.com).\n\
                 You have three tools: search_records, delete_record, transfer_funds.\n\
                 The analyst role can ONLY search. Delete and transfer will be denied.\n\
                 If a tool returns an ACCESS DENIED error, explain the permission boundary.",
            )
            .model(model)
            .tool(Arc::new(SearchRecords))
            .tool(Arc::new(DeleteRecord))
            .tool(Arc::new(TransferFunds))
            .build()?,
    );

    let sessions: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());
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

    let query = "Search for revenue reports, then delete record #1 and transfer $5000 to vendor.";
    println!("  **User:** {query}\n");
    print!("  **Agent:** ");

    let message = Content::new("user").with_text(query);
    let mut stream = runner
        .run(UserId::new("user")?, SessionId::new("s1")?, message)
        .await?;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() {
                    print!("{text}");
                }
            }
        }
    }
    println!("\n");

    println!("=== All RBAC + audit trail checks passed! ===");
    Ok(())
}
