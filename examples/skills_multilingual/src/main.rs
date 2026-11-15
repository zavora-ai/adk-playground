//! Multilingual Skill Matching Example
//!
//! Demonstrates Unicode-aware skill selection across Chinese, Russian, and English.
//! The agent dynamically selects the most relevant skill based on the user's query
//! language and injects the skill's instructions into the prompt context.
//!
//! # Running
//!
//! ```bash
//! export GOOGLE_API_KEY=your-key
//! cargo run --manifest-path examples/skills_multilingual/Cargo.toml
//! ```

use adk_rust::prelude::*;
use adk_skill::{
    SelectionPolicy, SkillIndex, apply_skill_injection, load_skill_index, select_skills,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("═══════════════════════════════════════════════════════════════");
    println!("  Multilingual Skill Matching — Unicode-Aware Agent Selection");
    println!("═══════════════════════════════════════════════════════════════");
    println!();

    // Load skill index from .skills/ directory relative to this example
    let skills_root = std::env::current_dir()?.join("examples/skills_multilingual");
    let index = load_skill_index(&skills_root)?;

    println!("📚 Loaded {} skills from {}", index.len(), skills_root.display());
    println!();
    for skill in index.skills() {
        println!("   • {} — {}", skill.name, skill.description);
    }
    println!();

    let policy = SelectionPolicy { top_k: 3, min_score: 0.1, ..SelectionPolicy::default() };

    // ─── Scenario 1: Chinese query → Database skill ─────────────────────
    demonstrate_query(&index, &policy, "查询数据库中最近一周的订单性能", "Chinese database query");

    // ─── Scenario 2: Chinese query → Desktop control skill ──────────────
    demonstrate_query(&index, &policy, "截图看看屏幕上有什么应用在运行", "Chinese desktop query");

    // ─── Scenario 3: Russian query → Document search skill ──────────────
    demonstrate_query(
        &index,
        &policy,
        "найти документы о политике безопасности",
        "Russian document search query",
    );

    // ─── Scenario 4: English query → Code review skill ──────────────────
    demonstrate_query(
        &index,
        &policy,
        "review the latest pull request for security issues",
        "English code review query",
    );

    // ─── Scenario 5: Mixed query → Best match wins ──────────────────────
    demonstrate_query(
        &index,
        &policy,
        "帮我 review 这个 SQL query 的性能",
        "Mixed Chinese+English query",
    );

    // ─── Scenario 6: Japanese query → Task management ───────────────────
    demonstrate_query(
        &index,
        &policy,
        "タスクの優先度を変更して期限を明日に設定",
        "Japanese task management query",
    );

    // ─── Scenario 7: Korean query → Calendar management ─────────────────
    demonstrate_query(
        &index,
        &policy,
        "내일 일정 확인하고 충돌 있는지 알려줘",
        "Korean calendar query",
    );

    // ─── Scenario 8: Agent with injected skill context ──────────────────
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  Scenario 8: Full Agent with Skill Injection");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();

    let api_key = std::env::var("GOOGLE_API_KEY")
        .or_else(|_| std::env::var("OPENAI_API_KEY"))
        .unwrap_or_default();

    if api_key.is_empty() {
        println!("  ⚠️  No API key set — skipping live agent demo.");
        println!("     Set GOOGLE_API_KEY or OPENAI_API_KEY to run this scenario.");
    } else {
        run_agent_with_skill(&index, &api_key).await?;
    }

    println!();
    println!("✅ Example completed successfully.");
    Ok(())
}

fn demonstrate_query(index: &SkillIndex, policy: &SelectionPolicy, query: &str, label: &str) {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  {label}");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();
    println!("  Query: \"{query}\"");
    println!();

    let matches = select_skills(index, query, policy);

    if matches.is_empty() {
        println!("  ❌ No matching skills found.");
    } else {
        for (i, m) in matches.iter().enumerate() {
            println!(
                "  {}. {} (score: {:.2}) — tools: {:?}",
                i + 1,
                m.skill.name,
                m.score,
                m.skill.allowed_tools,
            );
        }
        println!();
        println!("  ✅ Selected: {}", matches[0].skill.name);

        // Demonstrate injection
        let mut content = Content::new("user").with_text(query);
        let injected = apply_skill_injection(&mut content, index, policy, 2000);
        if let Some(m) = injected {
            println!("  📝 Injected skill '{}' into prompt context", m.skill.name);
        }
    }
    println!();
}

async fn run_agent_with_skill(index: &SkillIndex, api_key: &str) -> anyhow::Result<()> {
    let model = GeminiModel::new(api_key, "gemini-2.5-flash")?;

    // Build the agent with skill injection
    let agent = LlmAgentBuilder::new("multilingual-assistant")
        .instruction(
            "You are a multilingual assistant. You have been given a specialized skill \
             based on the user's query. Follow the skill's instructions precisely. \
             Respond in the same language as the user's query.",
        )
        .model(Arc::new(model))
        .build()?;

    // Simulate a Chinese database query with skill injection
    let query = "分析 orders 表最近7天的查询性能，找出慢查询";
    let policy = SelectionPolicy { top_k: 1, min_score: 0.1, ..SelectionPolicy::default() };

    let mut content = Content::new("user").with_text(query);
    let injected = apply_skill_injection(&mut content, index, &policy, 2000);

    if let Some(m) = &injected {
        println!("  📝 Selected skill: {} (score: {:.2})", m.skill.name, m.score);
        println!("  🔧 Available tools: {:?}", m.skill.allowed_tools);
        println!();
    }

    println!("  💬 Query: \"{query}\"");
    println!();

    // Run the agent
    use adk_rust::session::{CreateRequest, SessionService};
    use futures::StreamExt;

    let sessions: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());
    let session = sessions
        .create(CreateRequest {
            app_name: "skills-demo".to_string(),
            user_id: "user-1".to_string(),
            session_id: None,
            state: Default::default(),
        })
        .await?;

    let runner = Runner::builder()
        .app_name("skills-demo")
        .agent(Arc::new(agent))
        .session_service(sessions)
        .build()?;

    let mut stream = runner.run_str("user-1", session.id(), content).await?;

    print!("  🤖 Response: ");
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Part::Text { text } = part {
                    print!("{text}");
                }
            }
        }
    }
    println!();

    Ok(())
}
