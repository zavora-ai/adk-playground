use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Deserialize, JsonSchema)]
struct ProductQuery {
    /// Product name to look up
    product: String,
}

/// Look up product details, pricing, and stock levels.
#[tool]
async fn lookup_product(args: ProductQuery) -> adk_tool::Result<serde_json::Value> {
    let info = match args.product.to_lowercase().as_str() {
        "macbook pro" | "macbook" => serde_json::json!({
            "product": "MacBook Pro 14\"", "price": 1999.00, "stock": 23,
            "specs": "M3 Pro, 18GB RAM, 512GB SSD", "category": "Laptops"
        }),
        "airpods" | "airpods pro" => serde_json::json!({
            "product": "AirPods Pro 2", "price": 249.00, "stock": 156,
            "specs": "Active Noise Cancellation, USB-C", "category": "Audio"
        }),
        "ipad" | "ipad air" => serde_json::json!({
            "product": "iPad Air M2", "price": 599.00, "stock": 0,
            "specs": "11\" Liquid Retina, M2 chip, 128GB", "category": "Tablets",
            "restock_date": "March 25, 2026"
        }),
        "keyboard" | "magic keyboard" => serde_json::json!({
            "product": "Magic Keyboard", "price": 299.00, "stock": 42,
            "specs": "Touch ID, Numeric Keypad, USB-C", "category": "Accessories"
        }),
        _ => serde_json::json!({
            "error": format!("Product '{}' not found. Available: MacBook Pro, AirPods Pro, iPad Air, Magic Keyboard", args.product)
        }),
    };
    Ok(info)
}

#[derive(Deserialize, JsonSchema)]
struct CartItem {
    /// Product name to add
    product: String,
    /// Quantity to add
    quantity: u64,
}

/// Add a product to the shopping cart.
#[tool]
async fn add_to_cart(args: CartItem) -> adk_tool::Result<serde_json::Value> {
    let unit_price = match args.product.to_lowercase().as_str() {
        "macbook pro" | "macbook" => 1999.00,
        "airpods" | "airpods pro" => 249.00,
        "ipad" | "ipad air" => 599.00,
        "keyboard" | "magic keyboard" => 299.00,
        _ => 0.00,
    };
    Ok(serde_json::json!({
        "product": args.product,
        "quantity": args.quantity,
        "unit_price": unit_price,
        "total": unit_price * args.quantity as f64,
        "cart_id": "CART-8821",
        "status": "added"
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("shop_assistant")
            .instruction(
                "You are a helpful shopping assistant for an electronics store.\n\
                 - Use lookup_product to find product details, pricing, and stock levels.\n\
                 - Use add_to_cart to add items when the customer wants to buy.\n\
                 - Always check product availability before adding to cart.\n\
                 - If something is out of stock, mention the restock date if available.\n\
                 - Keep responses concise and helpful.",
            )
            .model(model)
            .tool(Arc::new(LookupProduct))
            .tool(Arc::new(AddToCart))
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

    // Turn 1: Browse products
    println!("👤 User: I'm looking for a new laptop and some earbuds. What do you have?\n");
    let msg1 = Content::new("user")
        .with_text("I'm looking for a new laptop and some earbuds. What do you have?");
    let mut stream = runner
        .run(UserId::new("user")?, SessionId::new("s1")?, msg1)
        .await?;
    print!("🤖 Assistant: ");
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
    println!("\n");

    // Turn 2: Follow-up referencing Turn 1 context + new action
    println!(
        "👤 User: Nice! Add the laptop and 2 AirPods to my cart. Also, is the iPad available?\n"
    );
    let msg2 = Content::new("user")
        .with_text("Nice! Add the laptop and 2 AirPods to my cart. Also, is the iPad available?");
    let mut stream = runner
        .run(UserId::new("user")?, SessionId::new("s1")?, msg2)
        .await?;
    print!("🤖 Assistant: ");
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
    println!("\n");

    // Turn 3: References all prior context
    println!("👤 User: What's my cart total so far?\n");
    let msg3 = Content::new("user").with_text("What's my cart total so far?");
    let mut stream = runner
        .run(UserId::new("user")?, SessionId::new("s1")?, msg3)
        .await?;
    print!("🤖 Assistant: ");
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
