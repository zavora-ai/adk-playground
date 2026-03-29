use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_rust::tool::AgentTool;
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

// ── Mock Tools ──────────────────────────────────────────────

#[derive(Deserialize, JsonSchema)]
struct AccountLookup {
    /// Customer account ID or email
    customer_id: String,
}

/// Look up a customer's account details and recent billing history.
#[tool]
async fn lookup_account(args: AccountLookup) -> adk_tool::Result<serde_json::Value> {
    Ok(serde_json::json!({
        "customer_id": args.customer_id,
        "name": "Alex Johnson",
        "plan": "Business ($79/mo)",
        "status": "active",
        "member_since": "2024-03-15",
        "recent_charges": [
            { "date": "2026-03-01", "amount": 79.00, "description": "Business Plan - March 2026" },
            { "date": "2026-03-01", "amount": 79.00, "description": "Business Plan - March 2026 (DUPLICATE)" },
            { "date": "2026-02-01", "amount": 79.00, "description": "Business Plan - February 2026" }
        ],
        "payment_method": "Visa ending in 4242"
    }))
}

#[derive(Deserialize, JsonSchema)]
struct RefundRequest {
    /// Customer account ID
    customer_id: String,
    /// Amount to refund in USD
    amount: f64,
    /// Reason for the refund
    reason: String,
}

/// Process a refund for a customer. Returns approval status.
#[tool]
async fn process_refund(args: RefundRequest) -> adk_tool::Result<serde_json::Value> {
    // Simulate: refunds over $50 need manager approval
    if args.amount > 50.0 {
        return Ok(serde_json::json!({
            "status": "pending_approval",
            "refund_id": "REF-20260320-001",
            "amount": args.amount,
            "reason": args.reason,
            "message": "Refund exceeds $50 limit. Manager approval required.",
            "requires": "manager_approval"
        }));
    }
    Ok(serde_json::json!({
        "status": "approved",
        "refund_id": "REF-20260320-001",
        "amount": args.amount,
        "reason": args.reason,
        "eta": "3-5 business days"
    }))
}

#[derive(Deserialize, JsonSchema)]
struct ApprovalDecision {
    /// Refund ID to approve or deny
    refund_id: String,
    /// Whether to approve the refund
    approved: bool,
    /// Manager's note explaining the decision
    note: String,
}

/// Manager tool: approve or deny a pending refund request.
#[tool]
async fn approve_refund(args: ApprovalDecision) -> adk_tool::Result<serde_json::Value> {
    if args.approved {
        Ok(serde_json::json!({
            "refund_id": args.refund_id,
            "status": "approved",
            "approved_by": "Manager",
            "note": args.note,
            "eta": "3-5 business days",
            "confirmation": "Customer will receive email confirmation within 1 hour."
        }))
    } else {
        Ok(serde_json::json!({
            "refund_id": args.refund_id,
            "status": "denied",
            "denied_by": "Manager",
            "note": args.note,
            "next_steps": "Customer may appeal via support ticket."
        }))
    }
}

// ── Agent Setup ─────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    // ── Tier 1: Billing Agent ──
    // Can look up accounts and initiate refunds, but refunds >$50 need escalation
    let billing_agent = LlmAgentBuilder::new("billing_agent")
        .description("Handles billing inquiries: account lookup, charges, and refund initiation. Cannot approve refunds over $50.")
        .instruction(
            "You are a billing specialist. Your job:\n\
             1. Look up customer accounts with lookup_account\n\
             2. Process refunds with process_refund\n\n\
             CRITICAL: If process_refund returns status 'pending_approval', you MUST respond with \
             EXACTLY this format: 'ESCALATION REQUIRED: Refund [refund_id] for $[amount] needs \
             manager approval. Reason: [reason]'. Do NOT resolve the issue yourself. \
             Do NOT tell the customer the refund is complete. Just report the escalation need."
        )
        .model(model.clone())
        .tool(Arc::new(LookupAccount))
        .tool(Arc::new(ProcessRefund))
        .build()?;

    // ── Tier 2: Manager Agent ──
    // Has authority to approve/deny refunds that exceed the billing agent's limit
    let manager_agent = LlmAgentBuilder::new("manager_agent")
        .description("Manager who reviews and approves/denies escalated refund requests over $50. Must use approve_refund tool.")
        .instruction(
            "You are a customer service manager. You MUST use the approve_refund tool to \
             make your decision.\n\n\
             When you receive an escalation:\n\
             1. Use approve_refund with the refund_id, set approved=true for duplicate charges\n\
             2. Always include a note explaining your decision\n\n\
             You MUST call approve_refund — do not just respond with text."
        )
        .model(model.clone())
        .tool(Arc::new(ApproveRefund))
        .build()?;

    // Wrap agents as callable tools so the coordinator gets results back
    // and can orchestrate multi-step workflows
    let billing_tool = AgentTool::new(Arc::new(billing_agent)).timeout(Duration::from_secs(30));
    let manager_tool = AgentTool::new(Arc::new(manager_agent)).timeout(Duration::from_secs(30));

    // ── Coordinator ──
    // Uses agents-as-tools to orchestrate the full resolution flow
    let coordinator = Arc::new(
        LlmAgentBuilder::new("coordinator")
            .instruction(
                "You coordinate customer service by delegating to your team. You NEVER handle \
                 issues directly — you ALWAYS delegate to the appropriate agent.\n\n\
                 Your team (call them as tools):\n\
                 - billing_agent: Looks up accounts and initiates refunds\n\
                 - manager_agent: Approves/denies refunds over $50\n\n\
                 MANDATORY WORKFLOW — follow these steps IN ORDER:\n\
                 Step 1: Call billing_agent with the customer's issue\n\
                 Step 2: If billing_agent mentions 'ESCALATION REQUIRED' or 'pending_approval', \
                 you MUST call manager_agent with the refund details (refund_id, amount, reason)\n\
                 Step 3: After manager_agent responds, summarize the full resolution to the customer: \
                 account lookup → refund initiated → manager approved → expected timeline.\n\n\
                 IMPORTANT: You must call BOTH billing_agent AND manager_agent for refunds over $50. \
                 Do not skip the manager step. Do not stop after billing_agent."
            )
            .model(model)
            .tool(Arc::new(billing_tool))
            .tool(Arc::new(manager_tool))
            .max_iterations(10)
            .build()?
    );

    // ── Session & Runner ────────────────────────────────────

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
        agent: coordinator,
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

    // ── Customer Interaction ────────────────────────────────

    println!("╔══════════════════════════════════════════════════╗");
    println!("║       Customer Service — Escalation Demo        ║");
    println!("║  Coordinator → Billing Agent → Manager Agent    ║");
    println!("╚══════════════════════════════════════════════════╝\n");

    // Customer reports a billing problem — should trigger full escalation flow:
    // coordinator → billing_agent (lookup + refund → pending) → manager_agent (approve) → summary
    println!("👤 Customer: I see two charges of $79 on March 1st for my Business Plan. \
              My account is alex@example.com. Please refund the duplicate and confirm it's approved.\n");

    let msg = Content::new("user").with_text(
        "I see two charges of $79 on March 1st for my Business Plan. \
             My account is alex@example.com. Please refund the duplicate charge \
             and get it fully approved so I know it's done.",
    );
    let mut stream = runner
        .run(UserId::new("user")?, SessionId::new("s1")?, msg)
        .await?;

    print!("🤖 Resolution: ");
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
