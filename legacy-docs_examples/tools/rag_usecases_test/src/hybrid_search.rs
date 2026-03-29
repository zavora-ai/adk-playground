//! Hybrid search with custom reranker — combining vector + keyword scoring
//!
//! Demonstrates: custom Reranker implementation, keyword boosting,
//! metadata filtering, and pipeline integration with reranking.

use adk_rag::{
    Document, EmbeddingProvider, FixedSizeChunker, InMemoryVectorStore,
    RagConfig, RagPipeline, Reranker, SearchResult,
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

/// Hybrid reranker that combines vector similarity with keyword matching
/// and metadata boosting. This is a common production pattern.
struct HybridReranker {
    /// Weight for keyword matches (added per matching keyword)
    keyword_boost: f32,
    /// Weight for metadata field matches
    metadata_boost: f32,
    /// Metadata field to check for boosting
    boost_field: String,
    /// Values in the boost field that get extra score
    boost_values: Vec<String>,
}

#[async_trait::async_trait]
impl Reranker for HybridReranker {
    async fn rerank(
        &self,
        query: &str,
        mut results: Vec<SearchResult>,
    ) -> adk_rag::Result<Vec<SearchResult>> {
        let keywords: Vec<String> = query
            .split_whitespace()
            .filter(|w| w.len() > 3)
            .map(|w| w.to_lowercase())
            .collect();

        for r in &mut results {
            let text_lower = r.chunk.text.to_lowercase();

            // Keyword boost: exact word matches in chunk text
            let keyword_hits = keywords.iter()
                .filter(|kw| text_lower.contains(kw.as_str()))
                .count();
            r.score += keyword_hits as f32 * self.keyword_boost;

            // Metadata boost: prioritize chunks from preferred sources
            if let Some(field_val) = r.chunk.metadata.get(&self.boost_field) {
                if self.boost_values.iter().any(|v| v == field_val) {
                    r.score += self.metadata_boost;
                }
            }
        }

        // Re-sort by combined score
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        Ok(results)
    }
}

/// Metadata filter reranker — removes results that don't match criteria
struct MetadataFilterReranker {
    required_field: String,
    required_value: String,
}

#[async_trait::async_trait]
impl Reranker for MetadataFilterReranker {
    async fn rerank(
        &self,
        _query: &str,
        results: Vec<SearchResult>,
    ) -> adk_rag::Result<Vec<SearchResult>> {
        Ok(results.into_iter().filter(|r| {
            r.chunk.metadata.get(&self.required_field)
                .map(|v| v == &self.required_value)
                .unwrap_or(false)
        }).collect())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Hybrid Search with Reranking ===\n");

    // Build pipeline with hybrid reranker
    let reranker = Arc::new(HybridReranker {
        keyword_boost: 0.15,
        metadata_boost: 0.2,
        boost_field: "source".to_string(),
        boost_values: vec!["official-docs".to_string()],
    });

    let pipeline = RagPipeline::builder()
        .config(RagConfig::builder().chunk_size(256).chunk_overlap(50).top_k(5).build()?)
        .embedding_provider(Arc::new(HashEmbedder))
        .vector_store(Arc::new(InMemoryVectorStore::new()))
        .chunker(Arc::new(FixedSizeChunker::new(256, 50)))
        .reranker(reranker)
        .build()?;

    pipeline.create_collection("kb").await?;

    // Ingest docs from different sources
    pipeline.ingest_batch("kb", &[
        Document {
            id: "official-1".into(),
            text: "To configure authentication, set the API_KEY environment variable. \
                   The key must be a valid 40-character hexadecimal string.".into(),
            metadata: HashMap::from([("source".into(), "official-docs".into())]),
            source_uri: None,
        },
        Document {
            id: "blog-1".into(),
            text: "I found that setting the API key in the environment works great. \
                   Just export API_KEY=your-key-here and you're good to go.".into(),
            metadata: HashMap::from([("source".into(), "community-blog".into())]),
            source_uri: None,
        },
        Document {
            id: "official-2".into(),
            text: "Rate limiting is configured per API key. Default limits are 1000 \
                   requests per minute. Contact support to increase limits.".into(),
            metadata: HashMap::from([("source".into(), "official-docs".into())]),
            source_uri: None,
        },
        Document {
            id: "forum-1".into(),
            text: "Has anyone figured out the API key rate limits? I keep getting 429 errors \
                   when making too many requests.".into(),
            metadata: HashMap::from([("source".into(), "forum".into())]),
            source_uri: None,
        },
    ]).await?;
    println!("✓ Ingested 4 documents from 3 sources");

    // Query — hybrid reranker should boost official docs + keyword matches
    let results = pipeline.query("kb", "API key configuration").await?;
    println!("\nQuery: 'API key configuration'");
    for (i, r) in results.iter().enumerate() {
        let source = r.chunk.metadata.get("source").map(|s| s.as_str()).unwrap_or("?");
        let preview = r.chunk.text.chars().take(60).collect::<String>();
        println!("  [{i}] score={:.3} source={source} → \"{preview}...\"", r.score);
    }

    // Official docs should rank higher due to metadata boost
    if results.len() >= 2 {
        let top_source = results[0].chunk.metadata.get("source").map(|s| s.as_str());
        println!("\n✓ Top result source: {:?}", top_source);
    }

    // 2. Metadata filter reranker — strict filtering
    println!("\n## Metadata Filtering\n");

    let filter_reranker = MetadataFilterReranker {
        required_field: "source".to_string(),
        required_value: "official-docs".to_string(),
    };

    // Manually apply filter to demonstrate the pattern
    let all_results = pipeline.query("kb", "API key").await?;
    let filtered = filter_reranker.rerank("API key", all_results).await?;
    println!("Before filter: results from all sources");
    println!("After filter:  {} results (official-docs only)", filtered.len());
    for r in &filtered {
        let source = r.chunk.metadata.get("source").map(|s| s.as_str()).unwrap_or("?");
        assert_eq!(source, "official-docs");
    }
    println!("✓ MetadataFilterReranker keeps only official-docs");

    // Cleanup
    pipeline.delete_collection("kb").await?;
    println!("\n✓ Collection cleaned up");

    println!("\n=== All hybrid search tests passed! ===");
    Ok(())
}
