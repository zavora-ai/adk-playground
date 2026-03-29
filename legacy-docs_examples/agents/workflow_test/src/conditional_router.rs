//! LLM-based Conditional Router Example
//!
//! Uses LlmConditionalAgent to intelligently route queries based on LLM classification.

use adk_rust::prelude::*;
use adk_rust::Launcher;
use std::sync::Arc;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-2.5-flash")?);

    // Create specialist agents (wrapped in Arc for sharing)
    let tech_agent: Arc<dyn Agent> = Arc::new(
        LlmAgentBuilder::new("tech_expert")
            .instruction("You are a senior software engineer. Answer with code examples, \
                         technical depth, and best practices. Be precise and thorough.")
            .model(model.clone())
            .build()?
    );

    let general_agent: Arc<dyn Agent> = Arc::new(
        LlmAgentBuilder::new("general_helper")
            .instruction("You are a friendly general assistant. Explain things simply \
                         without jargon. Use analogies. Be warm and conversational.")
            .model(model.clone())
            .build()?
    );

    let creative_agent: Arc<dyn Agent> = Arc::new(
        LlmAgentBuilder::new("creative_writer")
            .instruction("You are a creative writer. Be imaginative, expressive, and \
                         engaging. Use vivid language and storytelling techniques.")
            .model(model.clone())
            .build()?
    );

    // Create LLM-based conditional router
    let router = LlmConditionalAgent::builder("smart_router", model.clone())
        .instruction("Analyze the user's question and classify it as exactly ONE of: \
                     'technical' (coding, debugging, architecture, programming), \
                     'general' (facts, knowledge, how-to, everyday questions), \
                     'creative' (writing, stories, brainstorming, imagination). \
                     Respond with ONLY the category name, nothing else.")
        .route("technical", tech_agent)
        .route("general", general_agent.clone())
        .route("creative", creative_agent)
        .default_route(general_agent)
        .build()?;

    println!("🧠 LLM-Powered Intelligent Router");
    println!("   The LLM classifies your question and routes to:");
    println!("   • 'technical' → Tech Expert (code, debugging)");
    println!("   • 'general' → General Helper (facts, how-to)");
    println!("   • 'creative' → Creative Writer (stories, imagination)");
    println!();
    println!("Example prompts:");
    println!("   • 'How do I fix a borrow error in Rust?' → technical");
    println!("   • 'What is the capital of France?' → general");
    println!("   • 'Write me a haiku about the moon' → creative");
    println!();

    Launcher::new(Arc::new(router)).run().await?;
    Ok(())
}
