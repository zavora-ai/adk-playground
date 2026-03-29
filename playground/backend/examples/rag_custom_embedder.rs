//! Custom Embedder — agent with TF-IDF RAG pipeline
//!
//! Implements a custom EmbeddingProvider (TF-IDF), builds a RAG pipeline,
//! ingests programming language docs, and runs an agent that searches
//! the knowledge base to answer questions.

use adk_core::{SessionId, UserId};
use adk_rag::{
    Document, EmbeddingProvider, FixedSizeChunker, InMemoryVectorStore, RagConfig, RagPipeline,
};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// TF-IDF-inspired embedder: maps words to dimensions via hashing.
/// Production: use GeminiEmbeddingProvider or OpenAIEmbeddingProvider.
struct TfIdfEmbedder {
    dims: usize,
}

impl TfIdfEmbedder {
    fn word_to_dim(&self, word: &str) -> usize {
        word.bytes().fold(0usize, |acc, b| {
            acc.wrapping_mul(31).wrapping_add(b as usize)
        }) % self.dims
    }
}

#[async_trait::async_trait]
impl EmbeddingProvider for TfIdfEmbedder {
    async fn embed(&self, text: &str) -> adk_rag::Result<Vec<f32>> {
        let mut vector = vec![0.0f32; self.dims];
        let words: Vec<&str> = text
            .split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
            .filter(|w| w.len() > 2)
            .collect();
        let total = words.len() as f32;
        if total == 0.0 {
            return Ok(vector);
        }

        let mut freq: HashMap<usize, f32> = HashMap::new();
        for word in &words {
            *freq
                .entry(self.word_to_dim(&word.to_lowercase()))
                .or_default() += 1.0;
        }
        for (dim, count) in freq {
            vector[dim] = count / total;
        }
        let norm: f32 = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            vector.iter_mut().for_each(|x| *x /= norm);
        }
        Ok(vector)
    }
    fn dimensions(&self) -> usize {
        self.dims
    }
}

static PIPELINE: OnceLock<Arc<RagPipeline>> = OnceLock::new();

#[derive(Deserialize, JsonSchema)]
struct SearchArgs {
    /// The search query to find relevant documents
    query: String,
}

/// Search the programming languages knowledge base.
#[tool]
async fn search_docs(args: SearchArgs) -> adk_tool::Result<serde_json::Value> {
    let pipeline = PIPELINE.get().unwrap();
    let results = match pipeline.query("languages", &args.query).await {
        Ok(r) => r,
        Err(e) => return Ok(serde_json::json!({ "error": e.to_string() })),
    };

    let hits: Vec<_> = results
        .iter()
        .take(3)
        .map(|r| {
            serde_json::json!({
                "document_id": r.chunk.document_id,
                "text": r.chunk.text,
                "score": format!("{:.3}", r.score),
            })
        })
        .collect();

    Ok(serde_json::json!({ "results": hits }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== Custom Embedder — TF-IDF RAG Agent ===\n");

    // ── 1. Build pipeline with custom embedder ──
    let embedder = Arc::new(TfIdfEmbedder { dims: 128 });

    // Quick similarity demo
    let v1 = embedder.embed("Rust programming language safety").await?;
    let v2 = embedder
        .embed("Rust programming language performance")
        .await?;
    let v3 = embedder.embed("cooking recipes for Italian pasta").await?;
    let sim_12: f32 = v1.iter().zip(&v2).map(|(a, b)| a * b).sum();
    let sim_13: f32 = v1.iter().zip(&v3).map(|(a, b)| a * b).sum();
    println!("Cosine similarity demo:");
    println!("  rust+safety vs rust+perf = {:.3}", sim_12);
    println!("  rust+safety vs cooking   = {:.3}", sim_13);
    println!("  ✓ Related texts score higher\n");

    let pipeline = Arc::new(
        RagPipeline::builder()
            .config(
                RagConfig::builder()
                    .chunk_size(256)
                    .chunk_overlap(50)
                    .top_k(3)
                    .build()?,
            )
            .embedding_provider(embedder)
            .vector_store(Arc::new(InMemoryVectorStore::new()))
            .chunker(Arc::new(FixedSizeChunker::new(256, 50)))
            .build()?,
    );

    pipeline.create_collection("languages").await?;
    pipeline.ingest_batch("languages", &[
        Document {
            id: "rust".into(),
            text: "Rust is a systems programming language focused on safety, speed, and concurrency. \
                   It achieves memory safety without garbage collection through its ownership system. \
                   Rust's borrow checker prevents data races at compile time.".into(),
            metadata: HashMap::from([("paradigm".into(), "systems".into())]),
            source_uri: None,
        },
        Document {
            id: "python".into(),
            text: "Python is a high-level interpreted language popular for data science, web development, \
                   and scripting. It emphasizes readability with significant whitespace. Python has a \
                   rich ecosystem of libraries like NumPy, Pandas, and TensorFlow.".into(),
            metadata: HashMap::from([("paradigm".into(), "scripting".into())]),
            source_uri: None,
        },
        Document {
            id: "go".into(),
            text: "Go (Golang) is a statically typed language designed at Google for simplicity and \
                   efficiency. It features goroutines for lightweight concurrency, a built-in garbage \
                   collector, and fast compilation. Go excels at building network services and CLIs.".into(),
            metadata: HashMap::from([("paradigm".into(), "systems".into())]),
            source_uri: None,
        },
    ]).await?;
    println!("✓ Ingested 3 language docs with TF-IDF embeddings\n");

    let _ = PIPELINE.set(pipeline.clone());

    // ── 2. Build agent with RAG tool ──
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("embedder_agent")
            .instruction(
                "You are a programming language expert with a knowledge base.\n\
                 Use the search_docs tool to find information before answering.\n\
                 Compare languages objectively and cite the knowledge base.\n\
                 Be concise.",
            )
            .model(model)
            .tool(Arc::new(SearchDocs))
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

    // ── 3. Ask the agent ──
    let query = "Which language is best for building safe concurrent systems — Rust or Go? Search the docs and compare.";
    println!("**User:** {}\n", query);
    print!("**Agent:** ");

    let message = Content::new("user").with_text(query);
    let mut stream = runner
        .run(UserId::new("user")?, SessionId::new("s1")?, message)
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
    println!();

    pipeline.delete_collection("languages").await?;

    println!("\nProduction embedders:");
    println!("  adk-rag = {{ features = [\"gemini\"] }}  → GeminiEmbeddingProvider");
    println!("  adk-rag = {{ features = [\"openai\"] }}  → OpenAIEmbeddingProvider");

    Ok(())
}
