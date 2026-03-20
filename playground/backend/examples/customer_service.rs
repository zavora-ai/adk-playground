use adk_rust::prelude::*;
use adk_rust::session::{SessionService, CreateRequest};
use adk_rust::futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    // Specialist: Billing Agent
    let billing_agent = LlmAgentBuilder::new("billing_agent")
        .description("Handles billing: payments, invoices, subscriptions, refunds")
        .instruction(
            "You are a billing specialist. Help with:\n\
             - Invoice questions and payment history\n\
             - Subscription plans and upgrades\n\
             - Refund requests\n\
             Be professional and concise."
        )
        .model(model.clone())
        .build()?;

    // Specialist: Technical Support Agent
    let support_agent = LlmAgentBuilder::new("support_agent")
        .description("Handles technical support: bugs, errors, troubleshooting")
        .instruction(
            "You are a technical support specialist. Help with:\n\
             - Troubleshooting errors and bugs\n\
             - How-to questions\n\
             - Configuration and setup issues\n\
             Provide step-by-step guidance."
        )
        .model(model.clone())
        .build()?;

    // Coordinator routes to the right specialist
    let coordinator = Arc::new(
        LlmAgentBuilder::new("coordinator")
            .instruction(
                "You are a customer service coordinator. Route requests:\n\n\
                 - BILLING (payments, invoices, subscriptions) → billing_agent\n\
                 - TECHNICAL (errors, bugs, how-to) → support_agent\n\
                 - GENERAL greetings → respond yourself\n\n\
                 Briefly acknowledge the customer before transferring."
            )
            .model(model)
            .sub_agent(Arc::new(billing_agent))
            .sub_agent(Arc::new(support_agent))
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

    println!("=== Customer Service Multi-Agent ===");
    println!("Coordinator → Billing Agent | Support Agent\n");

    let message = Content::new("user")
        .with_text("I was charged twice on my last invoice and the app keeps crashing when I try to view it.");
    let mut stream = runner.run("user".into(), "s1".into(), message).await?;

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
