// Neo4j Sessions — Graph-powered session relationships
//
// Connects to a real Neo4j instance via `Neo4jSessionService`.
// Demonstrates: constraint/index migration, graph-node sessions,
// three-tier state as node properties, CRUD, and agent memory.
//
// Requires: NEO4J_URL + NEO4J_USER + NEO4J_PASS (+ GOOGLE_API_KEY for agent demo)

use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{
    CreateRequest, DeleteRequest, GetRequest, ListRequest, Neo4jSessionService, SessionService,
};
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let neo4j_url = std::env::var("NEO4J_URL").unwrap_or_else(|_| "bolt://localhost:7687".into());
    let neo4j_user = std::env::var("NEO4J_USER").unwrap_or_else(|_| "neo4j".into());
    let neo4j_pass = std::env::var("NEO4J_PASS").unwrap_or_else(|_| "adk_playground".into());

    println!("# Neo4j Session Backend\n");
    println!("Connecting to `{}`...\n", neo4j_url);

    // ── Connect & migrate ──
    let service = Neo4jSessionService::new(&neo4j_url, &neo4j_user, &neo4j_pass).await?;
    service.migrate().await?;
    println!("✅ Connected and constraints created\n");
    println!("Graph schema:");
    println!("  (:Session)-[:HAS_EVENT]->(:Event)");
    println!("  (:Session)-[:HAS_APP_STATE]->(:AppState)");
    println!("  (:Session)-[:HAS_USER_STATE]->(:UserState)\n");

    // ── Create sessions as graph nodes ──
    println!("## Creating Session Nodes\n");

    let mut state1 = HashMap::new();
    state1.insert("app:version".to_string(), serde_json::json!("2.1.0"));
    state1.insert("user:role".to_string(), serde_json::json!("researcher"));
    state1.insert(
        "user:department".to_string(),
        serde_json::json!("engineering"),
    );
    state1.insert("topic".to_string(), serde_json::json!("graph-databases"));

    let session1 = service
        .create(CreateRequest {
            app_name: "neo4j-demo".into(),
            user_id: "eve".into(),
            session_id: Some("neo4j-session-1".into()),
            state: state1,
        })
        .await?;
    println!("✅ Session node 1: `{}`", session1.id());
    println!("   user:role=researcher  topic=graph-databases");

    let mut state2 = HashMap::new();
    state2.insert("app:version".to_string(), serde_json::json!("2.1.0"));
    state2.insert("user:role".to_string(), serde_json::json!("analyst"));
    state2.insert("topic".to_string(), serde_json::json!("knowledge-graphs"));

    let session2 = service
        .create(CreateRequest {
            app_name: "neo4j-demo".into(),
            user_id: "frank".into(),
            session_id: Some("neo4j-session-2".into()),
            state: state2,
        })
        .await?;
    println!("✅ Session node 2: `{}`\n", session2.id());

    // ── List sessions ──
    println!("## Graph Queries\n");
    let eve_sessions = service
        .list(ListRequest {
            app_name: "neo4j-demo".into(),
            user_id: "eve".into(),
            limit: None,
            offset: None,
        })
        .await?;
    let frank_sessions = service
        .list(ListRequest {
            app_name: "neo4j-demo".into(),
            user_id: "frank".into(),
            limit: None,
            offset: None,
        })
        .await?;
    println!("Eve:   {} session node(s)", eve_sessions.len());
    println!("Frank: {} session node(s)\n", frank_sessions.len());

    // ── Run agent with Neo4j-backed sessions ──
    println!("## Agent with Neo4j Memory\n");

    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("neo4j_agent")
            .instruction(
                "You are a knowledge graph expert. Help users understand graph databases \
                 and their applications. Be concise (1-2 sentences).",
            )
            .model(model)
            .build()?,
    );

    let sessions_arc: Arc<dyn SessionService> = Arc::new(service);

    let runner = Runner::new(RunnerConfig {
        app_name: "neo4j-demo".into(),
        agent,
        session_service: sessions_arc.clone(),
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

    // Turn 1
    let msg1 = Content::new("user").with_text(
        "My name is Eve and I'm researching how graph databases model relationships. Remember that!"
    );
    print!("**User:** My name is Eve, researching graph DB relationships.\n\n**Agent:** ");
    let mut stream = runner
        .run(
            UserId::new("eve")?,
            SessionId::new("neo4j-session-1")?,
            msg1,
        )
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
    println!("\n");

    // Turn 2 — recall from Neo4j-persisted history
    let msg2 = Content::new("user").with_text("What's my name and what am I researching?");
    print!("**User:** What's my name and what am I researching?\n\n**Agent (recall):** ");
    let mut stream2 = runner
        .run(
            UserId::new("eve")?,
            SessionId::new("neo4j-session-1")?,
            msg2,
        )
        .await?;
    while let Some(event) = stream2.next().await {
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

    // ── Inspect graph node ──
    println!("## Node Inspection\n");
    let retrieved = sessions_arc
        .get(GetRequest {
            app_name: "neo4j-demo".into(),
            user_id: "eve".into(),
            session_id: "neo4j-session-1".into(),
            num_recent_events: None,
            after: None,
        })
        .await?;
    println!(
        "Session node `{}` — {} event nodes linked",
        retrieved.id(),
        retrieved.events().len()
    );
    let state_keys: Vec<_> = retrieved.state().all().keys().cloned().collect();
    println!("State properties: {:?}\n", state_keys);

    // ── Cleanup ──
    println!("## Cleanup\n");
    sessions_arc
        .delete(DeleteRequest {
            app_name: "neo4j-demo".into(),
            user_id: "eve".into(),
            session_id: "neo4j-session-1".into(),
        })
        .await?;
    sessions_arc
        .delete(DeleteRequest {
            app_name: "neo4j-demo".into(),
            user_id: "frank".into(),
            session_id: "neo4j-session-2".into(),
        })
        .await?;
    println!("✅ All test session nodes deleted\n");

    println!("---\n");
    println!("**Neo4j strengths:** Native graph relationships, Cypher queries, constraint-based schema, relationship traversal, knowledge graph integration.");

    Ok(())
}
