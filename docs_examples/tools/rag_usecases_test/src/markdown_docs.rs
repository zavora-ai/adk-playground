//! Markdown-aware RAG — chunking documentation with header preservation
//!
//! Demonstrates: MarkdownChunker for structured docs, header_path metadata,
//! section-aware retrieval, and documentation Q&A patterns.

use adk_rag::{
    Document, EmbeddingProvider, InMemoryVectorStore, MarkdownChunker,
    RagConfig, RagPipeline,
};
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
    println!("=== Markdown-Aware RAG ===\n");

    // Use MarkdownChunker instead of FixedSizeChunker for structured docs
    let pipeline = RagPipeline::builder()
        .config(RagConfig::builder().chunk_size(512).chunk_overlap(50).top_k(5).build()?)
        .embedding_provider(Arc::new(HashEmbedder))
        .vector_store(Arc::new(InMemoryVectorStore::new()))
        .chunker(Arc::new(MarkdownChunker::new(512, 50)))
        .build()?;

    pipeline.create_collection("docs").await?;

    // Ingest a structured markdown document
    let readme = Document {
        id: "adk-readme".into(),
        text: r#"# ADK-Rust

A comprehensive agent development kit for Rust.

## Installation

Add to your Cargo.toml:

```toml
[dependencies]
adk-rust = "0.4"
```

Then run `cargo build` to compile.

## Quick Start

Create your first agent:

```rust
use adk_rust::prelude::*;

let agent = LlmAgentBuilder::new("my-agent")
    .instruction("You are helpful.")
    .model(model)
    .build()?;
```

## Configuration

### Environment Variables

Set your API key:

- `GOOGLE_API_KEY` — for Gemini models
- `OPENAI_API_KEY` — for OpenAI models
- `ANTHROPIC_API_KEY` — for Claude models

### Feature Flags

ADK uses feature flags to control compilation:

- `minimal` — agents + Gemini + runner (~30s build)
- `standard` — + tools, sessions, memory (~51s build)
- `full` — everything including server, CLI, graph (~2min build)

## Troubleshooting

### Build Errors

If you see linker errors, ensure you have OpenSSL installed:

```bash
# macOS
brew install openssl

# Ubuntu
apt-get install libssl-dev
```

### Runtime Errors

Check that your API key is set and valid. Use `RUST_LOG=debug` for verbose output.
"#.into(),
        metadata: Default::default(),
        source_uri: Some("https://github.com/example/adk-rust/README.md".into()),
    };

    let chunks = pipeline.ingest("docs", &readme).await?;
    println!("✓ Ingested README: {} chunks", chunks.len());

    // Verify MarkdownChunker preserves header_path metadata
    let has_headers = chunks.iter().any(|c| c.metadata.contains_key("header_path"));
    assert!(has_headers, "MarkdownChunker should set header_path");
    println!("✓ Chunks have header_path metadata for section context");

    // Show chunk structure
    println!("\nChunk breakdown:");
    for (i, chunk) in chunks.iter().enumerate() {
        let header = chunk.metadata.get("header_path").map(|s| s.as_str()).unwrap_or("(root)");
        let preview = chunk.text.chars().take(60).collect::<String>().replace('\n', " ");
        println!("  [{i}] {header} → \"{preview}...\"");
    }

    // Query for specific sections
    println!("\n## Section-Aware Queries\n");

    let results = pipeline.query("docs", "how to install").await?;
    assert!(!results.is_empty());
    println!("Query 'how to install': {} results", results.len());
    let top = &results[0];
    println!("  Top result: score={:.3}", top.score);
    if let Some(header) = top.chunk.metadata.get("header_path") {
        println!("  Section: {}", header);
    }

    let results = pipeline.query("docs", "feature flags compilation").await?;
    println!("Query 'feature flags': {} results", results.len());

    let results = pipeline.query("docs", "openssl linker error fix").await?;
    println!("Query 'openssl linker error': {} results", results.len());

    // Cleanup
    pipeline.delete_collection("docs").await?;
    println!("\n✓ Collection cleaned up");

    println!("\n=== All markdown docs tests passed! ===");
    Ok(())
}
