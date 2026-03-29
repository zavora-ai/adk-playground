//! RagTool as an agent tool — validates the adk_core::Tool integration
//!
//! Demonstrates: RagTool construction, schema inspection, and using
//! RAG as a callable tool that agents can invoke during conversations.

use adk_rag::{
    Document, EmbeddingProvider, FixedSizeChunker, InMemoryVectorStore,
    RagConfig, RagPipeline, RagTool,
};
use adk_core::Tool;
use std::collections::HashMap;
use std::sync::Arc;

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== RagTool as Agent Tool ===\n");

    // Build pipeline
    let pipeline = Arc::new(
        RagPipeline::builder()
            .config(RagConfig::builder().chunk_size(256).chunk_overlap(50).top_k(3).build()?)
            .embedding_provider(Arc::new(HashEmbedder))
            .vector_store(Arc::new(InMemoryVectorStore::new()))
            .chunker(Arc::new(FixedSizeChunker::new(256, 50)))
            .build()?
    );

    // Ingest a knowledge base
    pipeline.create_collection("faq").await?;
    let docs = vec![
        Document {
            id: "setup".into(),
            text: "To install ADK-Rust, add adk-rust to your Cargo.toml dependencies. \
                   Run cargo build to compile. The minimum Rust version is 1.75.".into(),
            metadata: HashMap::from([("topic".into(), "installation".into())]),
            source_uri: None,
        },
        Document {
            id: "agents".into(),
            text: "Agents are the core abstraction in ADK. Use LlmAgentBuilder to create \
                   an agent with instructions, tools, and a model. Agents process user \
                   messages and return structured responses.".into(),
            metadata: HashMap::from([("topic".into(), "agents".into())]),
            source_uri: None,
        },
        Document {
            id: "tools".into(),
            text: "Tools extend agent capabilities. Define a struct implementing the Tool \
                   trait with name(), description(), parameters_schema(), and execute(). \
                   The #[tool] macro simplifies this to a single function annotation.".into(),
            metadata: HashMap::from([("topic".into(), "tools".into())]),
            source_uri: None,
        },
    ];
    pipeline.ingest_batch("faq", &docs).await?;
    println!("✓ Ingested {} documents into 'faq' collection", docs.len());

    // 1. Create RagTool
    let rag_tool = RagTool::new(pipeline.clone(), "faq");
    assert_eq!(rag_tool.name(), "rag_search");
    assert!(rag_tool.description().contains("knowledge base"));
    println!("✓ RagTool created: name='{}', default_collection='faq'", rag_tool.name());

    // 2. Inspect the tool schema (what the LLM sees)
    let schema = rag_tool.parameters_schema().unwrap();
    let props = schema["properties"].as_object().unwrap();
    assert!(props.contains_key("query"));
    assert!(props.contains_key("collection"));
    assert!(props.contains_key("top_k"));
    let required: Vec<&str> = schema["required"]
        .as_array().unwrap()
        .iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(required, vec!["query"]);
    println!("✓ Schema: query (required), collection (optional), top_k (optional)");

    // 3. The tool is ready to be attached to an agent:
    //    LlmAgentBuilder::new("assistant")
    //        .tool(Arc::new(rag_tool))
    //        .instruction("Use rag_search to find answers in the knowledge base.")
    //        .model(model)
    //        .build()?
    //
    // When the agent calls the tool, it sends:
    //   { "query": "how do I install ADK?", "collection": "faq", "top_k": 3 }
    //
    // The tool returns search results as JSON that the agent uses to answer.

    println!("\nAgent integration pattern:");
    println!("  LlmAgentBuilder::new(\"assistant\")");
    println!("      .tool(Arc::new(RagTool::new(pipeline, \"faq\")))");
    println!("      .instruction(\"Use rag_search to find answers.\")");
    println!("      .model(model)");
    println!("      .build()?");

    // Cleanup
    pipeline.delete_collection("faq").await?;
    println!("\n✓ Collection cleaned up");

    println!("\n=== All agent tool tests passed! ===");
    Ok(())
}
