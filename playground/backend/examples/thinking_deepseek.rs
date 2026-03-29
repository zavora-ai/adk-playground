use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use std::collections::HashMap;
use std::sync::Arc;

// ── DeepSeek Chain-of-Thought Reasoning ──
// DeepSeek's reasoner model (`deepseek-reasoner`) produces visible
// chain-of-thought reasoning before the final answer. Unlike OpenAI's
// internal reasoning, DeepSeek's thinking is streamed as `Part::Thinking`
// blocks — you can watch the model reason in real time.
//
// Key concepts:
//   - `DeepSeekConfig::reasoner()` — enables thinking mode
//   - `Part::Thinking` — contains the chain-of-thought text
//   - Thinking tokens are billed separately but improve accuracy
//   - Ideal for math, logic, coding, and multi-step problems
//
// This example presents a multi-step word problem that requires
// careful reasoning to avoid common pitfalls.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key =
        std::env::var("DEEPSEEK_API_KEY").expect("Set DEEPSEEK_API_KEY in your .env file");

    let model = Arc::new(DeepSeekClient::new(
        DeepSeekConfig::reasoner(api_key).with_max_tokens(4096),
    )?);

    let agent = Arc::new(
        LlmAgentBuilder::new("reasoning_engine")
            .instruction(
                "You are a precise reasoning engine. Show every step of your logic. \
                 Identify potential pitfalls and verify your answer before stating it.",
            )
            .model(model)
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

    println!("## 🧮 DeepSeek Chain-of-Thought — Watch the Model Think\n");

    // A tricky problem with a common wrong answer (many people say 10 cents)
    let prompt = "Solve this step by step:\n\n\
             A store sells notebooks and pens. A notebook costs $3 more than a pen. \
             If you buy 4 notebooks and 3 pens, the total is $39. \
             How much does each item cost?\n\n\
             Then verify: A train leaves City A at 9:00 AM traveling at 60 mph toward City B. \
             Another train leaves City B at 10:00 AM traveling at 90 mph toward City A. \
             The cities are 330 miles apart. At what time do the trains meet?";
    println!(
        "<!--USER_PROMPT_START-->\n{}\n<!--USER_PROMPT_END-->",
        prompt
    );
    let message = Content::new("user").with_text(prompt);
    let mut stream = runner
        .run(UserId::new("user")?, SessionId::new("s1")?, message)
        .await?;

    let mut thinking_tokens = 0;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                match part {
                    Part::Thinking { thinking, .. } => {
                        thinking_tokens += 1;
                        println!("<!--THINKING_START-->\n{}\n<!--THINKING_END-->", thinking);
                    }
                    _ => {
                        if let Some(text) = part.text() {
                            print!("{}", text);
                        }
                    }
                }
            }
        }
    }
    if thinking_tokens > 0 {
        println!("\n\n📊 Received {} thinking blocks", thinking_tokens);
    }
    println!();
    Ok(())
}
