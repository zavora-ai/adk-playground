use adk_core::{SessionId, UserId};
use adk_payments::domain::{
    Cart, CartLine, CommerceActor, CommerceActorRole, CommerceMode, FulfillmentKind,
    FulfillmentSelection, MerchantRef, Money, OrderSnapshot, OrderState, ProtocolDescriptor,
    ProtocolExtensions, ReceiptState, SafeTransactionSummary, TransactionId, TransactionRecord,
    TransactionState,
};
use adk_payments::guardrail::{
    AmountThresholdGuardrail, MerchantAllowlistGuardrail, PaymentPolicySet,
};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ── Shopping Agent — Agentic Commerce with Payment Tools ──
// An LLM agent that can browse products, add items to cart, check out,
// and look up transaction status — all through the adk-payments domain model.
//
// Key concepts:
//   - Agent tools return only `SafeTransactionSummary` (masked, transcript-safe)
//   - Raw payment credentials never appear in tool outputs
//   - Payment guardrails enforce amount limits and merchant allowlists
//   - The canonical transaction state machine prevents invalid transitions
//
// This example simulates a shopping flow with in-memory state.

// ── Shared state ──
struct ShopState {
    cart_items: Vec<CartLine>,
    transactions: HashMap<String, TransactionRecord>,
    policy_set: PaymentPolicySet,
}

static SHOP: std::sync::OnceLock<Arc<Mutex<ShopState>>> = std::sync::OnceLock::new();

fn shop() -> std::result::Result<Arc<Mutex<ShopState>>, serde_json::Value> {
    SHOP.get()
        .cloned()
        .ok_or_else(|| serde_json::json!({ "error": "Shop state not initialized" }))
}

fn lock_shop(
    binding: &Arc<Mutex<ShopState>>,
) -> std::result::Result<std::sync::MutexGuard<'_, ShopState>, serde_json::Value> {
    binding
        .lock()
        .map_err(|e| serde_json::json!({ "error": format!("Internal lock error: {e}") }))
}

// ── Product catalog ──
fn catalog() -> Vec<(&'static str, &'static str, i64)> {
    vec![
        ("SKU-LAPTOP", "Laptop Pro 16\"", 249_999),
        ("SKU-MOUSE", "Wireless Mouse", 4_999),
        ("SKU-KEYBOARD", "Mechanical Keyboard", 14_999),
        ("SKU-MONITOR", "4K Monitor 27\"", 59_999),
        ("SKU-HEADSET", "Noise-Canceling Headset", 29_999),
    ]
}

// ── Tools ──

#[derive(Deserialize, JsonSchema)]
struct BrowseArgs {
    /// Optional search term to filter products
    query: Option<String>,
}

/// Browse the product catalog. Returns available items with prices.
#[tool]
async fn browse_products(args: BrowseArgs) -> adk_tool::Result<serde_json::Value> {
    let products: Vec<serde_json::Value> = catalog()
        .iter()
        .filter(|(_, name, _)| {
            args.query
                .as_ref()
                .map_or(true, |q| name.to_lowercase().contains(&q.to_lowercase()))
        })
        .map(|(sku, name, price)| {
            serde_json::json!({
                "sku": sku, "name": name,
                "price": format!("${:.2}", *price as f64 / 100.0)
            })
        })
        .collect();
    Ok(serde_json::json!({ "products": products, "count": products.len() }))
}

#[derive(Deserialize, JsonSchema)]
struct AddToCartArgs {
    /// Product SKU to add
    sku: String,
    /// Quantity to add
    quantity: u32,
}

/// Add a product to the shopping cart.
#[tool]
async fn add_to_cart(args: AddToCartArgs) -> adk_tool::Result<serde_json::Value> {
    let products = catalog();
    let product = products.iter().find(|(s, _, _)| *s == args.sku);
    let Some((sku, name, price)) = product else {
        return Ok(serde_json::json!({ "error": format!("SKU {} not found", args.sku) }));
    };

    let line = CartLine {
        line_id: format!("line-{}", args.sku.to_lowercase()),
        merchant_sku: Some(sku.to_string()),
        title: name.to_string(),
        quantity: args.quantity,
        unit_price: Money::new("USD", *price, 2),
        total_price: Money::new("USD", price * args.quantity as i64, 2),
        product_class: Some("electronics".to_string()),
        extensions: ProtocolExtensions::default(),
    };

    let binding = match shop() {
        Ok(b) => b,
        Err(e) => return Ok(e),
    };
    let mut state = match lock_shop(&binding) {
        Ok(g) => g,
        Err(e) => return Ok(e),
    };
    state.cart_items.push(line);

    let total: i64 = state
        .cart_items
        .iter()
        .map(|l| l.total_price.amount_minor)
        .sum();
    let items: Vec<String> = state
        .cart_items
        .iter()
        .map(|l| format!("{} × {}", l.quantity, l.title))
        .collect();

    Ok(serde_json::json!({
        "added": name, "quantity": args.quantity,
        "cart_items": items,
        "cart_total": format!("${:.2}", total as f64 / 100.0)
    }))
}

#[derive(Deserialize, JsonSchema)]
struct CheckoutArgs {
    /// Fulfillment method: "shipping" or "digital"
    fulfillment: String,
}

/// Start checkout. Creates a canonical transaction, runs payment guardrails.
#[tool]
async fn checkout(args: CheckoutArgs) -> adk_tool::Result<serde_json::Value> {
    let binding = match shop() {
        Ok(b) => b,
        Err(e) => return Ok(e),
    };
    let mut state = match lock_shop(&binding) {
        Ok(g) => g,
        Err(e) => return Ok(e),
    };

    if state.cart_items.is_empty() {
        return Ok(serde_json::json!({ "error": "Cart is empty" }));
    }

    let total: i64 = state
        .cart_items
        .iter()
        .map(|l| l.total_price.amount_minor)
        .sum();
    let cart = Cart {
        cart_id: Some("cart-agent-1".to_string()),
        lines: state.cart_items.clone(),
        subtotal: Some(Money::new("USD", total, 2)),
        adjustments: Vec::new(),
        total: Money::new("USD", total, 2),
        affiliate_attribution: None,
        extensions: ProtocolExtensions::default(),
    };

    let tx_id = format!("tx-{}", chrono::Utc::now().timestamp_millis());
    let mut record = TransactionRecord::new(
        TransactionId::from(tx_id.as_str()),
        CommerceActor {
            actor_id: "shopping-agent".to_string(),
            role: CommerceActorRole::AgentSurface,
            display_name: Some("Shopping Assistant".to_string()),
            tenant_id: Some("tenant-demo".to_string()),
            extensions: ProtocolExtensions::default(),
        },
        MerchantRef {
            merchant_id: "merchant-techstore".to_string(),
            legal_name: "TechStore Inc.".to_string(),
            display_name: Some("TechStore".to_string()),
            statement_descriptor: Some("TECHSTORE*ONLINE".to_string()),
            country_code: Some("US".to_string()),
            website: Some("https://techstore.example".to_string()),
            extensions: ProtocolExtensions::default(),
        },
        CommerceMode::HumanPresent,
        cart,
        chrono::Utc::now(),
    );

    // Run payment guardrails
    let protocol = ProtocolDescriptor::acp("2026-01-30");
    let decision = state.policy_set.evaluate(&record, &protocol);

    if decision.is_deny() {
        let reasons: Vec<String> = decision
            .findings()
            .iter()
            .map(|f| f.reason.clone())
            .collect();
        return Ok(serde_json::json!({
            "status": "DENIED",
            "reasons": reasons,
            "transaction_id": tx_id,
        }));
    }

    let escalation = if decision.is_escalate() {
        let reasons: Vec<String> = decision
            .findings()
            .iter()
            .map(|f| f.reason.clone())
            .collect();
        Some(reasons)
    } else {
        None
    };

    // Progress through state machine
    record
        .transition_to(TransactionState::Negotiating, chrono::Utc::now())
        .ok();
    record
        .transition_to(TransactionState::AwaitingPaymentMethod, chrono::Utc::now())
        .ok();
    record
        .transition_to(TransactionState::Authorized, chrono::Utc::now())
        .ok();
    record
        .transition_to(TransactionState::Completed, chrono::Utc::now())
        .ok();

    // Attach fulfillment
    record.fulfillment = Some(FulfillmentSelection {
        fulfillment_id: "ful-1".to_string(),
        kind: if args.fulfillment == "digital" {
            FulfillmentKind::Digital
        } else {
            FulfillmentKind::Shipping
        },
        label: args.fulfillment.clone(),
        amount: if args.fulfillment == "shipping" {
            Some(Money::new("USD", 999, 2))
        } else {
            None
        },
        destination: None,
        requires_user_selection: false,
        extensions: ProtocolExtensions::default(),
    });

    record.order = Some(OrderSnapshot {
        order_id: Some(format!("order-{}", &tx_id[3..])),
        receipt_id: Some(format!("rcpt-{}", &tx_id[3..])),
        state: OrderState::Completed,
        receipt_state: ReceiptState::Settled,
        extensions: ProtocolExtensions::default(),
    });
    record.recompute_safe_summary();

    let summary = SafeTransactionSummary::from_record(&record);
    state.transactions.insert(tx_id.clone(), record);
    state.cart_items.clear();

    Ok(serde_json::json!({
        "status": "COMPLETED",
        "transaction_id": summary.transaction_id.as_str(),
        "merchant": summary.merchant_name,
        "items": summary.item_titles,
        "total": format!("${:.2}", summary.total.amount_minor as f64 / 100.0),
        "fulfillment": args.fulfillment,
        "escalation_warnings": escalation,
    }))
}

#[derive(Deserialize, JsonSchema)]
struct StatusArgs {
    /// Transaction ID to look up
    transaction_id: String,
}

/// Look up transaction status. Returns only the safe masked summary.
#[tool]
async fn transaction_status(args: StatusArgs) -> adk_tool::Result<serde_json::Value> {
    let binding = match shop() {
        Ok(b) => b,
        Err(e) => return Ok(e),
    };
    let state = match lock_shop(&binding) {
        Ok(g) => g,
        Err(e) => return Ok(e),
    };
    match state.transactions.get(&args.transaction_id) {
        Some(record) => {
            let summary = SafeTransactionSummary::from_record(record);
            Ok(serde_json::json!({
                "transaction_id": summary.transaction_id.as_str(),
                "merchant": summary.merchant_name,
                "items": summary.item_titles,
                "total": format!("${:.2}", summary.total.amount_minor as f64 / 100.0),
                "state": format!("{:?}", summary.state),
                "order_state": summary.order_state.map(|s| format!("{s:?}")),
                "receipt_state": summary.receipt_state.map(|s| format!("{s:?}")),
                "next_action": summary.next_required_action,
                "protocols": summary.protocol_tags,
            }))
        }
        None => Ok(serde_json::json!({ "error": "Transaction not found" })),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== 🛒 Shopping Agent — Agentic Commerce with Payment Tools ===\n");

    // Initialize shared state with guardrails
    let policy_set = PaymentPolicySet::new()
        .with(AmountThresholdGuardrail::new(Some(50_000), Some(500_000)))
        .with(MerchantAllowlistGuardrail::new(["merchant-techstore"]));

    let _ = SHOP.set(Arc::new(Mutex::new(ShopState {
        cart_items: Vec::new(),
        transactions: HashMap::new(),
        policy_set,
    })));

    println!("  Payment guardrails configured:");
    println!("    Amount: review > $500, deny > $5,000");
    println!("    Merchant allowlist: [merchant-techstore]\n");

    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("shopping_agent")
            .instruction(
                "You are a shopping assistant for TechStore. You help users browse products, \
                 add items to their cart, and complete checkout.\n\n\
                 Available tools:\n\
                 - browse_products: Search the product catalog\n\
                 - add_to_cart: Add items by SKU and quantity\n\
                 - checkout: Complete purchase (fulfillment: 'shipping' or 'digital')\n\
                 - transaction_status: Look up a completed transaction\n\n\
                 Payment guardrails enforce amount limits and merchant restrictions automatically. \
                 If checkout returns DENIED or escalation warnings, explain them to the user.\n\
                 Tool outputs show only safe masked summaries — raw payment data is never exposed.",
            )
            .model(model)
            .tool(Arc::new(BrowseProducts))
            .tool(Arc::new(AddToCart))
            .tool(Arc::new(Checkout))
            .tool(Arc::new(TransactionStatus))
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

    let query = "I'd like to buy a Laptop Pro and a Wireless Mouse. Ship them to me please.";
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

    println!("=== Shopping agent checkout complete! ===");
    Ok(())
}
