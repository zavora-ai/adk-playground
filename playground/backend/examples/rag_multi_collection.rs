//! Multi-Collection RAG — agent with domain-isolated knowledge bases
//!
//! Creates separate collections for engineering and HR docs, ingests them,
//! then builds an agent that queries the right collection to answer questions.

use adk_rust::prelude::*;
use adk_rust::session::{SessionService, CreateRequest};
use adk_rust::futures::StreamExt;
use adk_core::{UserId, SessionId};
use adk_rag::{
    Document, EmbeddingProvider, FixedSizeChunker, InMemoryVectorStore,
    RagConfig, RagPipeline,
};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// Simple hash-based embedder for demo (production: use GeminiEmbeddingProvider)
struct HashEmbedder;

#[async_trait::async_trait]
impl EmbeddingProvider for HashEmbedder {
    async fn embed(&self, text: &str) -> adk_rag::Result<Vec<f32>> {
        let hash = text.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
        let mut v = vec![0.0f32; 64];
        for (i, x) in v.iter_mut().enumerate() {
            *x = ((hash.wrapping_add(i as u64)) as f32).sin();
        }
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 { v.iter_mut().for_each(|x| *x /= norm); }
        Ok(v)
    }
    fn dimensions(&self) -> usize { 64 }
}

// We'll store the pipeline in a static for the tool to access
static PIPELINE: OnceLock<Arc<RagPipeline>> = OnceLock::new();

#[derive(Deserialize, JsonSchema)]
struct SearchArgs {
    /// The collection to search: "engineering" or "hr-policies"
    collection: String,
    /// The search query
    query: String,
}

/// Search a knowledge base collection. Returns matching document chunks.
#[tool]
async fn search_knowledge_base(args: SearchArgs) -> adk_tool::Result<serde_json::Value> {
    let pipeline = PIPELINE.get().unwrap();
    let results = match pipeline.query(&args.collection, &args.query).await {
        Ok(r) => r,
        Err(e) => return Ok(serde_json::json!({ "error": e.to_string() })),
    };

    let hits: Vec<_> = results.iter().take(3).map(|r| {
        serde_json::json!({
            "document_id": r.chunk.document_id,
            "text": r.chunk.text,
            "score": r.score,
        })
    }).collect();

    Ok(serde_json::json!({
        "collection": args.collection,
        "query": args.query,
        "results": hits,
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== Multi-Collection RAG Agent ===\n");

    // ── 1. Build RAG pipeline with two collections ──
    let pipeline = Arc::new(
        RagPipeline::builder()
            .config(RagConfig::builder().chunk_size(512).chunk_overlap(100).top_k(3).build()?)
            .embedding_provider(Arc::new(HashEmbedder))
            .vector_store(Arc::new(InMemoryVectorStore::new()))
            .chunker(Arc::new(FixedSizeChunker::new(512, 100)))
            .build()?
    );

    pipeline.create_collection("engineering").await?;
    pipeline.create_collection("hr-policies").await?;

    // Ingest engineering docs
    pipeline.ingest_batch("engineering", &[
        Document {
            id: "arch".into(),
            text: "Our microservices use gRPC for inter-service communication. Each service \
                   owns its database (PostgreSQL or Redis) and exposes a well-defined protobuf API. \
                   Services are deployed as Docker containers on Kubernetes.".into(),
            metadata: HashMap::from([("team".into(), "platform".into())]),
            source_uri: None,
        },
        Document {
            id: "deploy".into(),
            text: "Deployments use blue-green strategy with automatic rollback on error rate \
                   exceeding 1%. Canary releases target 5% traffic initially. All deployments \
                   require passing CI/CD pipeline with >80% test coverage.".into(),
            metadata: HashMap::from([("team".into(), "devops".into())]),
            source_uri: None,
        },
    ]).await?;

    // Ingest HR policies
    pipeline.ingest_batch("hr-policies", &[
        Document {
            id: "pto".into(),
            text: "Employees receive 20 days PTO per year. Unused PTO carries over up to 5 days. \
                   Requests require 2 weeks advance notice. Sick leave is separate and unlimited \
                   with doctor's note after 3 consecutive days.".into(),
            metadata: HashMap::from([("category".into(), "benefits".into())]),
            source_uri: None,
        },
        Document {
            id: "remote".into(),
            text: "Remote work policy: employees may work remotely up to 3 days per week. \
                   Full remote requires VP approval. Home office stipend of $500/year available. \
                   Core hours are 10am-3pm in your local timezone.".into(),
            metadata: HashMap::from([("category".into(), "workplace".into())]),
            source_uri: None,
        },
    ]).await?;
    println!("✓ Ingested docs into 'engineering' and 'hr-policies' collections\n");

    // Store pipeline for tool access
    let _ = PIPELINE.set(pipeline.clone());

    // ── 2. Build RAG-powered agent ──
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("rag_agent")
            .instruction(
                "You are a company knowledge assistant with access to two knowledge bases:\n\
                 - 'engineering': architecture, deployment, and technical docs\n\
                 - 'hr-policies': PTO, remote work, benefits, and workplace policies\n\n\
                 Use the search_knowledge_base tool to find relevant information before answering.\n\
                 Always cite which collection and document your answer comes from.\n\
                 Be concise and helpful."
            )
            .model(model)
            .tool(Arc::new(SearchKnowledgeBase))
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

    // ── 3. Ask the agent ──
    let query = "What's our deployment strategy and how many PTO days do employees get?";
    println!("**User:** {}\n", query);
    print!("**Agent:** ");

    let message = Content::new("user").with_text(query);
    let mut stream = runner.run(UserId::new("user")?, SessionId::new("s1")?, message).await?;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() { print!("{}", text); }
            }
        }
    }
    println!();

    // Cleanup
    pipeline.delete_collection("engineering").await?;
    pipeline.delete_collection("hr-policies").await?;

    Ok(())
}
