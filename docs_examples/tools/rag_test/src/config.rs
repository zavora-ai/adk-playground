//! RAG doc-test — validates config examples from rag.md

use adk_rag::RagConfig;

fn main() {
    println!("=== RAG Config Doc-Test ===\n");

    // From docs: Builder with custom values
    let config = RagConfig::builder()
        .chunk_size(256)
        .chunk_overlap(50)
        .top_k(5)
        .similarity_threshold(0.5)
        .build()
        .expect("valid config should build");

    assert_eq!(config.chunk_size, 256);
    assert_eq!(config.chunk_overlap, 50);
    assert_eq!(config.top_k, 5);
    assert!((config.similarity_threshold - 0.5).abs() < f32::EPSILON);
    println!("✓ Config builder with custom values works");

    // From docs: Default config
    let default = RagConfig::default();
    assert_eq!(default.chunk_size, 512);
    assert_eq!(default.chunk_overlap, 100);
    assert_eq!(default.top_k, 10);
    assert!((default.similarity_threshold - 0.0).abs() < f32::EPSILON);
    println!("✓ Default config has expected values");

    // Validation: overlap must be less than chunk_size
    let result = RagConfig::builder().chunk_size(100).chunk_overlap(100).build();
    assert!(result.is_err());
    println!("✓ Rejects overlap >= chunk_size");

    // Validation: top_k must be > 0
    let result = RagConfig::builder().top_k(0).build();
    assert!(result.is_err());
    println!("✓ Rejects top_k == 0");

    println!("\n=== All config tests passed! ===");
}
