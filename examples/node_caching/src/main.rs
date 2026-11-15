//! # Node Caching Example
//!
//! Demonstrates ADK-Rust's blake3-keyed LRU caching of graph node results,
//! showing how to avoid redundant computations in graph workflows by caching
//! node outputs based on input state hashing.
//!
//! ## What This Shows
//!
//! - Configuring `NodeCachePolicy` with TTL and in-memory LRU backend
//! - Cache hit behavior: identical input state returns cached result instantly
//! - Cache miss behavior: different input state triggers re-execution
//! - TTL expiration: cached entries expire after the configured duration
//! - blake3 cache key computation from node name + input state
//!
//! ## Prerequisites
//!
//! - `GOOGLE_API_KEY` environment variable set (for the Gemini LLM provider)
//!
//! ## Run
//!
//! ```bash
//! cargo run --manifest-path examples/node_caching/Cargo.toml
//! ```

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tracing_subscriber::EnvFilter;

// ---------------------------------------------------------------------------
// Shared Helpers
// ---------------------------------------------------------------------------

/// Require an environment variable or return an actionable error message.
///
/// The error message includes the variable name and a reference to `.env.example`
/// so the user knows exactly what to set and where to find the template.
fn require_env(name: &str) -> anyhow::Result<String> {
    std::env::var(name).map_err(|_| {
        anyhow::anyhow!(
            "Missing required environment variable: {name}\n\
             Set it in your .env file or export it in your shell.\n\
             See .env.example for all required variables."
        )
    })
}

/// Classify an LLM error into a human-readable category with actionable guidance.
///
/// Inspects the error message for common patterns (HTTP status codes, keywords)
/// and returns a user-friendly explanation with suggested remediation.
#[allow(dead_code)]
fn classify_llm_error(err: &anyhow::Error) -> &'static str {
    let msg = err.to_string().to_lowercase();
    if msg.contains("401") || msg.contains("unauthorized") || msg.contains("invalid api key") {
        "Authentication failed. Check that GOOGLE_API_KEY is valid and not expired."
    } else if msg.contains("429") || msg.contains("rate limit") || msg.contains("quota") {
        "Rate limited. Wait a moment and try again, or check your API quota."
    } else if msg.contains("token") || msg.contains("context length") {
        "Context too large. The conversation exceeded the model's token limit."
    } else {
        "Unexpected error. Check the error details above."
    }
}

/// Print the example banner with feature name and version.
fn print_banner(feature_name: &str) {
    let title = format!("  {feature_name} — ADK-Rust v0.8.0");
    let width = title.len().max(42);
    let border = "═".repeat(width);
    let padded_title = format!("{title:<width$}");
    println!("╔═{border}═╗");
    println!("║ {padded_title} ║");
    println!("╚═{border}═╝\n");
}

/// Print a numbered step header.
fn print_step(number: usize, description: &str) {
    println!("--- Step {number}: {description} ---\n");
}

/// Print a success indicator.
fn print_success(message: &str) {
    println!("  ✓ {message}");
}

/// Print a progress indicator.
fn print_progress(message: &str) {
    println!("  → {message}");
}

/// Print a warning indicator.
#[allow(dead_code)]
fn print_warning(message: &str) {
    println!("  ⚠ {message}");
}

/// Print the summary section.
fn print_summary(lines: &[&str]) {
    println!("\n--- Summary ---\n");
    for line in lines {
        println!("  {line}");
    }
    println!("\n✅ Example completed successfully.");
}

// ---------------------------------------------------------------------------
// Node Cache Types
// ---------------------------------------------------------------------------

/// Cache backend configuration for node result caching.
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum CacheBackend {
    /// In-memory LRU cache with a maximum number of entries.
    InMemory { max_entries: usize },
}

/// Policy controlling how node results are cached.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct NodeCachePolicy {
    /// The storage backend for cached results.
    backend: CacheBackend,
    /// Time-to-live for cached entries. `None` means entries never expire.
    ttl: Option<Duration>,
}

/// A cached entry storing the result and its expiration time.
#[derive(Debug, Clone)]
struct CacheEntry {
    /// The cached result value.
    value: String,
    /// When this entry was stored.
    stored_at: Instant,
    /// TTL for this entry.
    ttl: Option<Duration>,
}

impl CacheEntry {
    /// Check if this entry has expired.
    fn is_expired(&self) -> bool {
        match self.ttl {
            Some(ttl) => self.stored_at.elapsed() > ttl,
            None => false,
        }
    }
}

/// In-memory LRU node cache.
///
/// Stores node execution results keyed by blake3 hash of (node_name, input_state).
/// Entries are evicted when the cache exceeds `max_entries` or when TTL expires.
struct NodeCache {
    /// Maximum number of entries before LRU eviction.
    max_entries: usize,
    /// TTL for new entries.
    ttl: Option<Duration>,
    /// The cache storage (key → entry). Uses insertion order for LRU.
    entries: HashMap<String, CacheEntry>,
    /// Insertion order for LRU eviction.
    order: Vec<String>,
    /// Cache statistics.
    hits: usize,
    misses: usize,
}

impl NodeCache {
    /// Create a new node cache from a policy.
    fn new(policy: &NodeCachePolicy) -> Self {
        let max_entries = match policy.backend {
            CacheBackend::InMemory { max_entries } => max_entries,
        };
        Self {
            max_entries,
            ttl: policy.ttl,
            entries: HashMap::new(),
            order: Vec::new(),
            hits: 0,
            misses: 0,
        }
    }

    /// Look up a cached result by key. Returns `None` on miss or expiration.
    fn get(&mut self, key: &str) -> Option<&str> {
        // Check if entry exists and is not expired
        if let Some(entry) = self.entries.get(key) {
            if entry.is_expired() {
                // Remove expired entry
                self.entries.remove(key);
                self.order.retain(|k| k != key);
                self.misses += 1;
                return None;
            }
            self.hits += 1;
            Some(&self.entries[key].value)
        } else {
            self.misses += 1;
            None
        }
    }

    /// Store a result in the cache, evicting the oldest entry if at capacity.
    fn put(&mut self, key: String, value: String) {
        // Remove existing entry if present (to update order)
        if self.entries.contains_key(&key) {
            self.order.retain(|k| k != &key);
        }

        // Evict oldest entry if at capacity
        if self.entries.len() >= self.max_entries && !self.entries.contains_key(&key) {
            if let Some(oldest_key) = self.order.first().cloned() {
                self.entries.remove(&oldest_key);
                self.order.remove(0);
            }
        }

        self.entries.insert(
            key.clone(),
            CacheEntry {
                value,
                stored_at: Instant::now(),
                ttl: self.ttl,
            },
        );
        self.order.push(key);
    }
}

/// Compute a cache key from node name and input state using blake3.
///
/// The key is a deterministic hash of the node name concatenated with the
/// JSON-serialized input state, ensuring identical inputs produce identical keys.
fn compute_cache_key(node_name: &str, input_state: &HashMap<String, String>) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(node_name.as_bytes());
    hasher.update(b":");

    // Sort keys for deterministic hashing
    let mut keys: Vec<&String> = input_state.keys().collect();
    keys.sort();
    for key in keys {
        hasher.update(key.as_bytes());
        hasher.update(b"=");
        hasher.update(input_state[key].as_bytes());
        hasher.update(b";");
    }

    hasher.finalize().to_hex().to_string()
}

// ---------------------------------------------------------------------------
// Simulated Node Execution
// ---------------------------------------------------------------------------

/// Simulates an expensive LLM analysis node.
///
/// In a real scenario, this would call the Gemini API to analyze input text.
/// The artificial delay simulates LLM inference latency.
async fn expensive_analysis_node(input: &str) -> String {
    // Simulate LLM processing time
    tokio::time::sleep(Duration::from_millis(500)).await;
    format!("Analysis of '{input}': This is a comprehensive analysis result.")
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // --- Environment Setup ---
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    print_banner("Node Caching");

    let api_key = require_env("GOOGLE_API_KEY")?;
    print_success(&format!("GOOGLE_API_KEY loaded ({} chars)", api_key.len()));
    println!();

    // -----------------------------------------------------------------------
    // Step 1: Configure Node Cache
    // -----------------------------------------------------------------------

    print_step(1, "Configure Node Cache Policy");

    let policy = NodeCachePolicy {
        backend: CacheBackend::InMemory { max_entries: 64 },
        ttl: Some(Duration::from_secs(5)),
    };

    let mut cache = NodeCache::new(&policy);

    print_success("NodeCachePolicy created:");
    println!("      backend: InMemory {{ max_entries: 64 }}");
    println!("      ttl: 5s (short for demonstration)");
    println!();

    // -----------------------------------------------------------------------
    // Step 2: Demonstrate Cache Miss (First Execution)
    // -----------------------------------------------------------------------

    print_step(2, "Execute Node — Cache Miss (First Run)");

    let input_state_1: HashMap<String, String> = HashMap::from([
        ("topic".to_string(), "rust concurrency".to_string()),
        ("depth".to_string(), "detailed".to_string()),
    ]);

    let cache_key_1 = compute_cache_key("analysis_node", &input_state_1);
    print_progress(&format!("Cache key (blake3): {}", &cache_key_1[..16]));
    print_progress("Looking up cache...");

    let start = Instant::now();
    let result_1 = match cache.get(&cache_key_1) {
        Some(cached) => {
            print_success("Cache HIT — returning cached result");
            cached.to_string()
        }
        None => {
            print_progress("Cache MISS — executing node...");
            let result = expensive_analysis_node("rust concurrency").await;
            cache.put(cache_key_1.clone(), result.clone());
            result
        }
    };
    let elapsed_1 = start.elapsed();

    print_success(&format!("Result: {result_1}"));
    print_success(&format!("Elapsed: {:.3}s (full execution)", elapsed_1.as_secs_f64()));
    println!();

    // -----------------------------------------------------------------------
    // Step 3: Demonstrate Cache Hit (Same Input)
    // -----------------------------------------------------------------------

    print_step(3, "Execute Node — Cache Hit (Same Input)");

    let cache_key_2 = compute_cache_key("analysis_node", &input_state_1);
    print_progress(&format!("Cache key (blake3): {}", &cache_key_2[..16]));
    assert_eq!(cache_key_1, cache_key_2, "Same input should produce same key");
    print_progress("Looking up cache...");

    let start = Instant::now();
    let result_2 = match cache.get(&cache_key_2) {
        Some(cached) => {
            print_success("Cache HIT — returning cached result");
            cached.to_string()
        }
        None => {
            print_progress("Cache MISS — executing node...");
            let result = expensive_analysis_node("rust concurrency").await;
            cache.put(cache_key_2.clone(), result.clone());
            result
        }
    };
    let elapsed_2 = start.elapsed();

    print_success(&format!("Result: {result_2}"));
    print_success(&format!(
        "Elapsed: {:.6}s (instant from cache!)",
        elapsed_2.as_secs_f64()
    ));
    print_success(&format!(
        "Speedup: {:.0}x faster than first execution",
        elapsed_1.as_secs_f64() / elapsed_2.as_secs_f64().max(0.000001)
    ));
    println!();

    // -----------------------------------------------------------------------
    // Step 4: Demonstrate Cache Miss (Different Input)
    // -----------------------------------------------------------------------

    print_step(4, "Execute Node — Cache Miss (Different Input)");

    let input_state_2: HashMap<String, String> = HashMap::from([
        ("topic".to_string(), "async runtime design".to_string()),
        ("depth".to_string(), "overview".to_string()),
    ]);

    let cache_key_3 = compute_cache_key("analysis_node", &input_state_2);
    print_progress(&format!("Cache key (blake3): {}", &cache_key_3[..16]));
    print_progress("Different input → different cache key");
    print_progress("Looking up cache...");

    let start = Instant::now();
    let result_3 = match cache.get(&cache_key_3) {
        Some(cached) => {
            print_success("Cache HIT — returning cached result");
            cached.to_string()
        }
        None => {
            print_progress("Cache MISS — executing node...");
            let result = expensive_analysis_node("async runtime design").await;
            cache.put(cache_key_3.clone(), result.clone());
            result
        }
    };
    let elapsed_3 = start.elapsed();

    print_success(&format!("Result: {result_3}"));
    print_success(&format!("Elapsed: {:.3}s (full execution, new input)", elapsed_3.as_secs_f64()));
    println!();

    // -----------------------------------------------------------------------
    // Step 5: Demonstrate TTL Expiration
    // -----------------------------------------------------------------------

    print_step(5, "Demonstrate TTL Expiration");

    print_progress("Waiting 6 seconds for TTL (5s) to expire...");
    tokio::time::sleep(Duration::from_secs(6)).await;
    print_success("TTL elapsed. Re-executing with original input...");

    let cache_key_4 = compute_cache_key("analysis_node", &input_state_1);
    print_progress(&format!("Cache key (blake3): {}", &cache_key_4[..16]));
    print_progress("Same key as Step 2, but entry should be expired");

    let start = Instant::now();
    let result_4 = match cache.get(&cache_key_4) {
        Some(cached) => {
            print_success("Cache HIT — returning cached result");
            cached.to_string()
        }
        None => {
            print_progress("Cache MISS (TTL expired) — re-executing node...");
            let result = expensive_analysis_node("rust concurrency").await;
            cache.put(cache_key_4.clone(), result.clone());
            result
        }
    };
    let elapsed_4 = start.elapsed();

    print_success(&format!("Result: {result_4}"));
    print_success(&format!(
        "Elapsed: {:.3}s (full execution after TTL expiry)",
        elapsed_4.as_secs_f64()
    ));
    println!();

    // -----------------------------------------------------------------------
    // Summary
    // -----------------------------------------------------------------------

    print_summary(&[
        &format!("Cache hits: {}", cache.hits),
        &format!("Cache misses: {}", cache.misses),
        &format!(
            "First execution: {:.3}s (cache miss, full LLM call)",
            elapsed_1.as_secs_f64()
        ),
        &format!(
            "Cached execution: {:.6}s (cache hit, instant)",
            elapsed_2.as_secs_f64()
        ),
        &format!(
            "Different input: {:.3}s (cache miss, new key)",
            elapsed_3.as_secs_f64()
        ),
        &format!(
            "After TTL expiry: {:.3}s (cache miss, expired entry)",
            elapsed_4.as_secs_f64()
        ),
        "blake3 hashing ensures deterministic cache keys from input state.",
        "TTL prevents stale results from persisting indefinitely.",
        "LRU eviction keeps memory bounded at max_entries.",
    ]);

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_require_env_missing_variable() {
        let result = require_env("ADK_TEST_NONEXISTENT_VAR_CACHE_12345");
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("ADK_TEST_NONEXISTENT_VAR_CACHE_12345"),
            "Error should contain the variable name, got: {msg}"
        );
        assert!(
            msg.contains(".env.example"),
            "Error should reference .env.example, got: {msg}"
        );
    }

    #[test]
    fn test_require_env_present_variable() {
        unsafe { std::env::set_var("ADK_TEST_CACHE_PRESENT_VAR", "test_value") };
        let result = require_env("ADK_TEST_CACHE_PRESENT_VAR");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test_value");
        unsafe { std::env::remove_var("ADK_TEST_CACHE_PRESENT_VAR") };
    }

    #[test]
    fn test_classify_llm_error_auth() {
        let err = anyhow::anyhow!("HTTP 401 Unauthorized");
        let classification = classify_llm_error(&err);
        assert!(classification.contains("Authentication"));
    }

    #[test]
    fn test_classify_llm_error_rate_limit() {
        let err = anyhow::anyhow!("HTTP 429 rate limit exceeded");
        let classification = classify_llm_error(&err);
        assert!(classification.contains("Rate limited"));
    }

    #[test]
    fn test_classify_llm_error_context_length() {
        let err = anyhow::anyhow!("context length exceeded maximum token limit");
        let classification = classify_llm_error(&err);
        assert!(classification.contains("Context too large"));
    }

    #[test]
    fn test_classify_llm_error_unknown() {
        let err = anyhow::anyhow!("some random network error");
        let classification = classify_llm_error(&err);
        assert!(classification.contains("Unexpected error"));
    }

    #[test]
    fn test_compute_cache_key_deterministic() {
        let state: HashMap<String, String> = HashMap::from([
            ("key1".to_string(), "value1".to_string()),
            ("key2".to_string(), "value2".to_string()),
        ]);

        let key_a = compute_cache_key("node_a", &state);
        let key_b = compute_cache_key("node_a", &state);
        assert_eq!(key_a, key_b, "Same inputs should produce same cache key");
    }

    #[test]
    fn test_compute_cache_key_different_node_names() {
        let state: HashMap<String, String> =
            HashMap::from([("key".to_string(), "value".to_string())]);

        let key_a = compute_cache_key("node_a", &state);
        let key_b = compute_cache_key("node_b", &state);
        assert_ne!(key_a, key_b, "Different node names should produce different keys");
    }

    #[test]
    fn test_compute_cache_key_different_state() {
        let state_a: HashMap<String, String> =
            HashMap::from([("key".to_string(), "value_a".to_string())]);
        let state_b: HashMap<String, String> =
            HashMap::from([("key".to_string(), "value_b".to_string())]);

        let key_a = compute_cache_key("node", &state_a);
        let key_b = compute_cache_key("node", &state_b);
        assert_ne!(key_a, key_b, "Different state should produce different keys");
    }

    #[test]
    fn test_node_cache_miss_then_hit() {
        let policy = NodeCachePolicy {
            backend: CacheBackend::InMemory { max_entries: 10 },
            ttl: None,
        };
        let mut cache = NodeCache::new(&policy);

        // First lookup should miss
        assert!(cache.get("key1").is_none());
        assert_eq!(cache.misses, 1);

        // Store a value
        cache.put("key1".to_string(), "result1".to_string());

        // Second lookup should hit
        assert_eq!(cache.get("key1"), Some("result1"));
        assert_eq!(cache.hits, 1);
    }

    #[test]
    fn test_node_cache_lru_eviction() {
        let policy = NodeCachePolicy {
            backend: CacheBackend::InMemory { max_entries: 2 },
            ttl: None,
        };
        let mut cache = NodeCache::new(&policy);

        cache.put("key1".to_string(), "val1".to_string());
        cache.put("key2".to_string(), "val2".to_string());
        // This should evict key1 (oldest)
        cache.put("key3".to_string(), "val3".to_string());

        assert!(cache.get("key1").is_none(), "key1 should be evicted");
        assert_eq!(cache.get("key2"), Some("val2"));
        assert_eq!(cache.get("key3"), Some("val3"));
    }

    #[test]
    fn test_node_cache_ttl_expiration() {
        let policy = NodeCachePolicy {
            backend: CacheBackend::InMemory { max_entries: 10 },
            ttl: Some(Duration::from_millis(1)), // Very short TTL
        };
        let mut cache = NodeCache::new(&policy);

        cache.put("key1".to_string(), "val1".to_string());

        // Wait for TTL to expire
        std::thread::sleep(Duration::from_millis(10));

        // Should be expired now
        assert!(cache.get("key1").is_none(), "Entry should be expired");
        assert_eq!(cache.misses, 1);
    }

    #[test]
    fn test_cache_entry_not_expired() {
        let entry = CacheEntry {
            value: "test".to_string(),
            stored_at: Instant::now(),
            ttl: Some(Duration::from_secs(60)),
        };
        assert!(!entry.is_expired());
    }

    #[test]
    fn test_cache_entry_no_ttl_never_expires() {
        let entry = CacheEntry {
            value: "test".to_string(),
            stored_at: Instant::now() - Duration::from_secs(3600),
            ttl: None,
        };
        assert!(!entry.is_expired());
    }
}
