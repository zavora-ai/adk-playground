//! Custom embedding provider — implementing the EmbeddingProvider trait
//!
//! Demonstrates: trait implementation, TF-IDF-style embeddings,
//! dimension configuration, and integration with the pipeline.

use adk_rag::{
    Document, EmbeddingProvider, FixedSizeChunker, InMemoryVectorStore,
    RagConfig, RagPipeline,
};
use std::collections::HashMap;
use std::sync::Arc;

/// A simple TF-IDF-inspired embedding provider.
///
/// Maps each unique word to a dimension and uses term frequency as the value.
/// This is a teaching example — production systems should use neural embeddings
/// (GeminiEmbeddingProvider, OpenAIEmbeddingProvider, etc.).
struct TfIdfEmbedder {
    dims: usize,
}

impl TfIdfEmbedder {
    fn new(dims: usize) -> Self {
        Self { dims }
    }

    /// Hash a word to a dimension index (simple modular hashing)
    fn word_to_dim(&self, word: &str) -> usize {
        let hash = word.bytes().fold(0usize, |acc, b| acc.wrapping_mul(31).wrapping_add(b as usize));
        hash % self.dims
    }
}

#[async_trait::async_trait]
impl EmbeddingProvider for TfIdfEmbedder {
    async fn embed(&self, text: &str) -> adk_rag::Result<Vec<f32>> {
        let mut vector = vec![0.0f32; self.dims];

        // Count term frequencies
        let words: Vec<&str> = text.split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
            .filter(|w| w.len() > 2)
            .collect();

        let total = words.len() as f32;
        if total == 0.0 {
            return Ok(vector);
        }

        // Accumulate term frequencies into vector dimensions
        let mut freq: HashMap<usize, f32> = HashMap::new();
        for word in &words {
            let dim = self.word_to_dim(&word.to_lowercase());
            *freq.entry(dim).or_default() += 1.0;
        }

        for (dim, count) in freq {
            vector[dim] = count / total; // Normalized term frequency
        }

        // L2 normalize
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Custom Embedding Provider ===\n");

    // 1. Create custom embedder
    let embedder = Arc::new(TfIdfEmbedder::new(128));
    assert_eq!(embedder.dimensions(), 128);
    println!("✓ TfIdfEmbedder created with {} dimensions", embedder.dimensions());

    // 2. Test embedding directly
    let vec1 = embedder.embed("Rust programming language safety").await?;
    assert_eq!(vec1.len(), 128);
    let norm: f32 = vec1.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 0.01, "Vector should be L2-normalized");
    println!("✓ Embedding produces normalized 128-dim vector");

    // 3. Similar texts should have similar embeddings
    let vec2 = embedder.embed("Rust programming language performance").await?;
    let vec3 = embedder.embed("cooking recipes for Italian pasta").await?;

    let sim_12: f32 = vec1.iter().zip(&vec2).map(|(a, b)| a * b).sum();
    let sim_13: f32 = vec1.iter().zip(&vec3).map(|(a, b)| a * b).sum();
    println!("  Similarity(rust+safety, rust+performance) = {:.3}", sim_12);
    println!("  Similarity(rust+safety, cooking+pasta)    = {:.3}", sim_13);
    assert!(sim_12 > sim_13, "Related texts should be more similar");
    println!("✓ Related texts have higher cosine similarity");

    // 4. Integrate with pipeline
    let pipeline = RagPipeline::builder()
        .config(RagConfig::builder().chunk_size(256).chunk_overlap(50).top_k(3).build()?)
        .embedding_provider(embedder)
        .vector_store(Arc::new(InMemoryVectorStore::new()))
        .chunker(Arc::new(FixedSizeChunker::new(256, 50)))
        .build()?;

    pipeline.create_collection("test").await?;
    pipeline.ingest_batch("test", &[
        Document {
            id: "rust".into(),
            text: "Rust is a systems programming language focused on safety, speed, and concurrency.".into(),
            metadata: Default::default(),
            source_uri: None,
        },
        Document {
            id: "python".into(),
            text: "Python is a high-level interpreted language popular for data science and scripting.".into(),
            metadata: Default::default(),
            source_uri: None,
        },
    ]).await?;

    let results = pipeline.query("test", "systems programming safety").await?;
    assert!(!results.is_empty());
    assert_eq!(results[0].chunk.document_id, "rust");
    println!("✓ Pipeline with custom embedder: 'systems programming safety' → rust doc");

    pipeline.delete_collection("test").await?;
    println!("✓ Cleanup complete");

    // Production embedders available via feature flags:
    println!("\nProduction embedding providers:");
    println!("  adk-rag = {{ features = [\"gemini\"] }}  → GeminiEmbeddingProvider");
    println!("  adk-rag = {{ features = [\"openai\"] }}  → OpenAIEmbeddingProvider");

    println!("\n=== All custom embedder tests passed! ===");
    Ok(())
}
