//! Multi-collection RAG — separate knowledge domains with isolated search
//!
//! Demonstrates: creating multiple collections, ingesting domain-specific
//! documents, querying each independently, and cross-collection patterns.

use adk_rag::{
    Document, EmbeddingProvider, FixedSizeChunker, InMemoryVectorStore,
    RagConfig, RagPipeline,
};
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
    println!("=== Multi-Collection RAG ===\n");

    let pipeline = RagPipeline::builder()
        .config(RagConfig::builder().chunk_size(512).chunk_overlap(100).top_k(3).build()?)
        .embedding_provider(Arc::new(HashEmbedder))
        .vector_store(Arc::new(InMemoryVectorStore::new()))
        .chunker(Arc::new(FixedSizeChunker::new(512, 100)))
        .build()?;

    // 1. Create domain-specific collections
    pipeline.create_collection("engineering").await?;
    pipeline.create_collection("hr-policies").await?;
    pipeline.create_collection("product-docs").await?;
    println!("✓ Created 3 collections: engineering, hr-policies, product-docs");

    // 2. Ingest engineering docs
    pipeline.ingest_batch("engineering", &[
        Document {
            id: "arch-001".into(),
            text: "Our microservices architecture uses gRPC for inter-service communication. \
                   Each service owns its database and exposes a well-defined API contract.".into(),
            metadata: HashMap::from([("team".into(), "platform".into())]),
            source_uri: None,
        },
        Document {
            id: "deploy-001".into(),
            text: "Deployments use blue-green strategy with automatic rollback on error rate \
                   exceeding 1%. Canary releases target 5% of traffic for 30 minutes.".into(),
            metadata: HashMap::from([("team".into(), "devops".into())]),
            source_uri: None,
        },
    ]).await?;
    println!("✓ Ingested 2 engineering documents");

    // 3. Ingest HR policies
    pipeline.ingest_batch("hr-policies", &[
        Document {
            id: "pto-001".into(),
            text: "Employees receive 20 days of paid time off per year. Unused PTO carries \
                   over up to 5 days. PTO requests require manager approval 2 weeks in advance.".into(),
            metadata: HashMap::from([("category".into(), "benefits".into())]),
            source_uri: None,
        },
        Document {
            id: "remote-001".into(),
            text: "Remote work policy: employees may work remotely up to 3 days per week. \
                   Core hours are 10am-3pm in your local timezone for meetings.".into(),
            metadata: HashMap::from([("category".into(), "workplace".into())]),
            source_uri: None,
        },
    ]).await?;
    println!("✓ Ingested 2 HR policy documents");

    // 4. Ingest product docs
    pipeline.ingest("product-docs", &Document {
        id: "api-v2".into(),
        text: "API v2 introduces rate limiting at 1000 requests per minute per API key. \
               Batch endpoints accept up to 100 items. Authentication uses Bearer tokens.".into(),
        metadata: HashMap::from([("version".into(), "v2".into())]),
        source_uri: None,
    }).await?;
    println!("✓ Ingested 1 product document");

    // 5. Query each collection independently
    println!("\n## Collection-Specific Queries\n");

    let eng_results = pipeline.query("engineering", "deployment strategy").await?;
    assert!(!eng_results.is_empty());
    println!("Engineering query 'deployment strategy': {} results", eng_results.len());
    for r in &eng_results {
        println!("  score={:.3} text={}...", r.score, &r.chunk.text[..50.min(r.chunk.text.len())]);
    }

    let hr_results = pipeline.query("hr-policies", "vacation days").await?;
    assert!(!hr_results.is_empty());
    println!("HR query 'vacation days': {} results", hr_results.len());

    let prod_results = pipeline.query("product-docs", "rate limiting").await?;
    assert!(!prod_results.is_empty());
    println!("Product query 'rate limiting': {} results", prod_results.len());

    // 6. Cross-collection search pattern
    // Query all collections and merge results (useful for general Q&A bots)
    println!("\n## Cross-Collection Search\n");
    let query = "what are the policies";
    let mut all_results = Vec::new();
    for collection in &["engineering", "hr-policies", "product-docs"] {
        let mut results = pipeline.query(collection, query).await?;
        for r in &mut results {
            // Tag results with their source collection
            r.chunk.metadata.insert("_collection".into(), collection.to_string());
        }
        all_results.extend(results);
    }
    // Sort by score descending
    all_results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    println!("Cross-collection query '{}': {} total results", query, all_results.len());
    for r in all_results.iter().take(3) {
        let coll = r.chunk.metadata.get("_collection").map(|s| s.as_str()).unwrap_or("?");
        println!("  [{coll}] score={:.3}", r.score);
    }

    // 7. Cleanup
    pipeline.delete_collection("engineering").await?;
    pipeline.delete_collection("hr-policies").await?;
    pipeline.delete_collection("product-docs").await?;
    println!("\n✓ All collections cleaned up");

    println!("\n=== All multi-collection tests passed! ===");
    Ok(())
}
