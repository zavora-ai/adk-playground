use adk_core::{SessionId, UserId};
use adk_payments::domain::{
    Cart, CartLine, CommerceActor, CommerceActorRole, CommerceMode, EvidenceReference,
    FulfillmentKind, FulfillmentSelection, MerchantRef, Money, OrderSnapshot, OrderState,
    ProtocolDescriptor, ProtocolExtensionEnvelope, ProtocolExtensions, ReceiptState,
    SafeTransactionSummary, TransactionId, TransactionRecord, TransactionState,
};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ── Checkout Agent — AI-Driven Payment Lifecycle ──
// An LLM agent that drives the full ACP checkout lifecycle:
//   create session → update cart → select fulfillment → complete → verify status
//
// The agent operates through the canonical adk-payments domain model.
// Every tool returns only SafeTransactionSummary — raw payment artifacts
// are stored as evidence, never leaked into the conversation.
//
// Key concepts:
//   - TransactionRecord with enforced state machine (Draft → ... → Completed)
//   - Protocol extensions preserved losslessly across ACP and AP2
//   - Evidence references for immutable audit trail
//   - Minor-unit Money arithmetic (no floating-point drift)

// ── Shared transaction store ──
struct TxStore {
    current: Option<TransactionRecord>,
}

static STORE: std::sync::OnceLock<Arc<Mutex<TxStore>>> = std::sync::OnceLock::new();

fn store() -> std::result::Result<Arc<Mutex<TxStore>>, serde_json::Value> {
    STORE
        .get()
        .cloned()
        .ok_or_else(|| serde_json::json!({ "error": "Transaction store not initialized" }))
}

fn lock_store(
    binding: &Arc<Mutex<TxStore>>,
) -> std::result::Result<std::sync::MutexGuard<'_, TxStore>, serde_json::Value> {
    binding
        .lock()
        .map_err(|e| serde_json::json!({ "error": format!("Internal lock error: {e}") }))
}

fn format_money(m: &Money) -> String {
    format!(
        "${:.2} {}",
        m.amount_minor as f64 / 10f64.powi(m.scale as i32),
        m.currency
    )
}

// ── Tools ──

#[derive(Deserialize, JsonSchema)]
struct CreateSessionArgs {
    /// Merchant name for this checkout
    merchant_name: String,
    /// Comma-separated list of items with quantities, e.g. "Laptop Pro x1, USB-C Hub x2"
    items: String,
}

/// Create a new checkout session with a cart. Returns a masked transaction summary.
#[tool]
async fn create_checkout_session(args: CreateSessionArgs) -> adk_tool::Result<serde_json::Value> {
    // Parse items from the description
    let mut lines = Vec::new();
    let mut total_minor: i64 = 0;

    let known_prices: HashMap<&str, i64> = HashMap::from([
        ("laptop pro", 249_999),
        ("usb-c hub", 4_999),
        ("wireless mouse", 4_999),
        ("mechanical keyboard", 14_999),
        ("4k monitor", 59_999),
        ("headset", 29_999),
        ("phone case", 1_999),
        ("charger", 3_499),
    ]);

    for (i, segment) in args.items.split(',').enumerate() {
        let segment = segment.trim().to_lowercase();
        let (name, qty) = if let Some(pos) = segment.rfind(" x") {
            let q: u32 = segment[pos + 2..].trim().parse().unwrap_or(1);
            (segment[..pos].trim().to_string(), q)
        } else {
            (segment.clone(), 1)
        };

        let unit_price = known_prices
            .iter()
            .find(|(k, _)| name.contains(*k))
            .map(|(_, v)| *v)
            .unwrap_or(9_999);

        let line_total = unit_price * qty as i64;
        total_minor += line_total;

        lines.push(CartLine {
            line_id: format!("line-{}", i + 1),
            merchant_sku: Some(format!("SKU-{}", i + 1)),
            title: name.clone(),
            quantity: qty,
            unit_price: Money::new("USD", unit_price, 2),
            total_price: Money::new("USD", line_total, 2),
            product_class: Some("electronics".to_string()),
            extensions: ProtocolExtensions::default(),
        });
    }

    let cart = Cart {
        cart_id: Some("cart-checkout-1".to_string()),
        lines,
        subtotal: Some(Money::new("USD", total_minor, 2)),
        adjustments: Vec::new(),
        total: Money::new("USD", total_minor, 2),
        affiliate_attribution: None,
        extensions: ProtocolExtensions::default(),
    };

    let tx_id = format!("tx-{}", chrono::Utc::now().timestamp_millis());
    let mut record = TransactionRecord::new(
        TransactionId::from(tx_id.as_str()),
        CommerceActor {
            actor_id: "checkout-agent".to_string(),
            role: CommerceActorRole::AgentSurface,
            display_name: Some("Checkout Agent".to_string()),
            tenant_id: Some("tenant-demo".to_string()),
            extensions: ProtocolExtensions::default(),
        },
        MerchantRef {
            merchant_id: "merchant-1".to_string(),
            legal_name: format!("{} LLC", args.merchant_name),
            display_name: Some(args.merchant_name.clone()),
            statement_descriptor: Some(format!("{}*ONLINE", args.merchant_name.to_uppercase())),
            country_code: Some("US".to_string()),
            website: None,
            extensions: ProtocolExtensions::default(),
        },
        CommerceMode::HumanPresent,
        cart,
        chrono::Utc::now(),
    );

    // Attach ACP protocol extension
    record.attach_extension(
        ProtocolExtensionEnvelope::new(ProtocolDescriptor::acp("2026-01-30"))
            .with_field("checkoutMode", serde_json::json!("agent-initiated")),
    );

    // Move to Negotiating
    record
        .transition_to(TransactionState::Negotiating, chrono::Utc::now())
        .ok();

    let summary = SafeTransactionSummary::from_record(&record);
    let binding = match store() {
        Ok(b) => b,
        Err(e) => return Ok(e),
    };
    let mut guard = match lock_store(&binding) {
        Ok(g) => g,
        Err(e) => return Ok(e),
    };
    guard.current = Some(record);

    Ok(serde_json::json!({
        "status": "SESSION_CREATED",
        "transaction_id": summary.transaction_id.as_str(),
        "merchant": summary.merchant_name,
        "items": summary.item_titles,
        "total": format_money(&summary.total),
        "state": format!("{:?}", summary.state),
        "next_action": summary.next_required_action,
        "protocols": summary.protocol_tags,
    }))
}

#[derive(Deserialize, JsonSchema)]
struct FulfillmentArgs {
    /// Fulfillment type: "shipping", "digital", "pickup"
    method: String,
}

/// Select fulfillment method for the current checkout session.
#[tool]
async fn select_fulfillment(args: FulfillmentArgs) -> adk_tool::Result<serde_json::Value> {
    let binding = match store() {
        Ok(b) => b,
        Err(e) => return Ok(e),
    };
    let mut s = match lock_store(&binding) {
        Ok(g) => g,
        Err(e) => return Ok(e),
    };
    let Some(record) = s.current.as_mut() else {
        return Ok(serde_json::json!({ "error": "No active checkout session" }));
    };

    let kind = match args.method.to_lowercase().as_str() {
        "digital" => FulfillmentKind::Digital,
        "pickup" => FulfillmentKind::Pickup,
        _ => FulfillmentKind::Shipping,
    };
    let shipping_cost = match kind {
        FulfillmentKind::Shipping => Some(Money::new("USD", 999, 2)),
        _ => None,
    };

    record.fulfillment = Some(FulfillmentSelection {
        fulfillment_id: "ful-1".to_string(),
        kind: kind.clone(),
        label: args.method.clone(),
        amount: shipping_cost.clone(),
        destination: None,
        requires_user_selection: false,
        extensions: ProtocolExtensions::default(),
    });

    // Move to awaiting payment
    record
        .transition_to(TransactionState::AwaitingPaymentMethod, chrono::Utc::now())
        .ok();

    let summary = SafeTransactionSummary::from_record(record);
    Ok(serde_json::json!({
        "status": "FULFILLMENT_SELECTED",
        "method": args.method,
        "shipping_cost": shipping_cost.as_ref().map(|m| format_money(m)),
        "state": format!("{:?}", summary.state),
        "next_action": summary.next_required_action,
    }))
}

#[derive(Deserialize, JsonSchema)]
struct CompleteArgs {
    /// Payment method hint: "card", "delegated", "wallet"
    payment_method: String,
}

/// Complete the checkout — authorize and finalize the transaction.
#[tool]
async fn complete_checkout(args: CompleteArgs) -> adk_tool::Result<serde_json::Value> {
    let binding = match store() {
        Ok(b) => b,
        Err(e) => return Ok(e),
    };
    let mut s = match lock_store(&binding) {
        Ok(g) => g,
        Err(e) => return Ok(e),
    };
    let Some(record) = s.current.as_mut() else {
        return Ok(serde_json::json!({ "error": "No active checkout session" }));
    };

    // Authorize
    record
        .transition_to(TransactionState::Authorized, chrono::Utc::now())
        .ok();

    // Attach evidence reference (simulated payment receipt)
    record.attach_evidence_ref(EvidenceReference {
        evidence_id: format!("ev-pay-{}", chrono::Utc::now().timestamp_millis()),
        protocol: ProtocolDescriptor::acp("2026-01-30"),
        artifact_kind: "payment_authorization".to_string(),
        digest: Some("sha256:a1b2c3d4e5f6...".to_string()),
    });

    // Complete
    record
        .transition_to(TransactionState::Completed, chrono::Utc::now())
        .ok();

    // Attach order
    let tx_id = record.transaction_id.as_str().to_string();
    record.order = Some(OrderSnapshot {
        order_id: Some(format!("order-{}", &tx_id)),
        receipt_id: Some(format!("rcpt-{}", &tx_id)),
        state: OrderState::Completed,
        receipt_state: ReceiptState::Settled,
        extensions: ProtocolExtensions::default(),
    });
    record.recompute_safe_summary();

    let summary = SafeTransactionSummary::from_record(record);
    Ok(serde_json::json!({
        "status": "COMPLETED",
        "transaction_id": summary.transaction_id.as_str(),
        "merchant": summary.merchant_name,
        "items": summary.item_titles,
        "total": format_money(&summary.total),
        "payment_method": args.payment_method,
        "order_state": summary.order_state.map(|s| format!("{s:?}")),
        "receipt_state": summary.receipt_state.map(|s| format!("{s:?}")),
        "evidence_count": record.evidence_refs.len(),
        "note": "Raw payment artifacts stored as evidence, NOT in conversation",
    }))
}

#[derive(Deserialize, JsonSchema)]
struct StatusArgs {
    /// Any question about the current transaction
    question: String,
}

/// Check the current transaction status and summary.
#[tool]
async fn check_status(_args: StatusArgs) -> adk_tool::Result<serde_json::Value> {
    let binding = match store() {
        Ok(b) => b,
        Err(e) => return Ok(e),
    };
    let s = match lock_store(&binding) {
        Ok(g) => g,
        Err(e) => return Ok(e),
    };
    let Some(record) = s.current.as_ref() else {
        return Ok(serde_json::json!({ "error": "No active checkout session" }));
    };

    let summary = SafeTransactionSummary::from_record(record);
    Ok(serde_json::json!({
        "transaction_id": summary.transaction_id.as_str(),
        "merchant": summary.merchant_name,
        "items": summary.item_titles,
        "total": format_money(&summary.total),
        "state": format!("{:?}", summary.state),
        "order_state": summary.order_state.map(|s| format!("{s:?}")),
        "receipt_state": summary.receipt_state.map(|s| format!("{s:?}")),
        "next_action": summary.next_required_action,
        "protocols": summary.protocol_tags,
        "evidence_refs": record.evidence_refs.len(),
        "transcript_summary": summary.transcript_text(),
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== 💳 Checkout Agent — AI-Driven Payment Lifecycle ===\n");

    let _ = STORE.set(Arc::new(Mutex::new(TxStore { current: None })));

    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("checkout_agent")
            .instruction(
                "You are a checkout agent that drives the full payment lifecycle.\n\n\
                 Your tools:\n\
                 - create_checkout_session: Start a new checkout with merchant and items\n\
                 - select_fulfillment: Choose shipping, digital, or pickup delivery\n\
                 - complete_checkout: Authorize and finalize the payment\n\
                 - check_status: View the current transaction summary\n\n\
                 Walk through the full checkout flow step by step:\n\
                 1. Create the checkout session with the requested items\n\
                 2. Select the appropriate fulfillment method\n\
                 3. Complete the checkout with a payment method\n\
                 4. Verify the final transaction status\n\n\
                 After each step, briefly explain what happened and the current state.\n\
                 Tool outputs show only safe masked summaries — raw payment credentials \
                 and authorization tokens are stored as evidence artifacts, never in the conversation."
            )
            .model(model)
            .tool(Arc::new(CreateCheckoutSession))
            .tool(Arc::new(SelectFulfillment))
            .tool(Arc::new(CompleteCheckout))
            .tool(Arc::new(CheckStatus))
            .build()?
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

    let query = "I want to buy a Laptop Pro and 2 USB-C Hubs from TechStore. \
                 Ship them to me and pay with card.";
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

    println!("=== Checkout lifecycle complete! ===");
    Ok(())
}
