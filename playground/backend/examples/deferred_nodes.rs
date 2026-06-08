//! # Deferred Nodes Example
//!
//! Demonstrates ADK-Rust's scatter-gather fan-in barriers — parallel upstream
//! paths feeding into a deferred node that waits for all (or some) results
//! before executing its aggregation logic.
//!
//! ## What This Shows
//!
//! - Configuring a graph with multiple parallel upstream paths
//! - Using `MergeStrategy::Collect` to gather all outputs into a vector
//! - Using `MergeStrategy::MergeMap` to merge upstream state maps
//! - Configuring `fan_in_timeout` for handling slow upstream paths
//! - A realistic parallel research scenario with LLM-powered branches
//!
//! ## Prerequisites
//!
//! - `GOOGLE_API_KEY` environment variable set (for the Gemini LLM provider)
//!
//! ## Run
//!
//! ```bash
//! cargo run --manifest-path examples/deferred_nodes/Cargo.toml
//! ```

use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;

// ===========================================================================
// Deferred Node API Types (simulating the runtime-reliability-sprint API)
// ===========================================================================

/// Strategy for merging multiple upstream outputs at a fan-in barrier.
///
/// When a deferred node has multiple upstream paths, this enum determines
/// how the outputs from those paths are combined before the deferred node
/// executes its own logic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeStrategy {
    /// Gather all upstream outputs into a `Vec<serde_json::Value>`.
    /// Order matches the order in which upstream paths complete.
    Collect,

    /// Merge upstream outputs (each expected to be a JSON object/map)
    /// into a single flattened map. Later keys overwrite earlier ones.
    MergeMap,
}

impl fmt::Display for MergeStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MergeStrategy::Collect => write!(f, "Collect"),
            MergeStrategy::MergeMap => write!(f, "MergeMap"),
        }
    }
}

/// Configuration for a deferred (fan-in) node in a graph workflow.
///
/// A deferred node waits for all (or a subset of) upstream parallel paths
/// to complete before executing. The `merge_strategy` determines how the
/// upstream outputs are combined, and `fan_in_timeout` sets the maximum
/// time to wait for all upstream paths.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeferredNodeConfig {
    /// How to combine upstream outputs.
    pub merge_strategy: MergeStrategy,

    /// Maximum time to wait for all upstream paths to complete.
    /// If `None`, the deferred node waits indefinitely.
    /// If `Some(duration)`, paths that haven't completed by the deadline
    /// are excluded from the merged result.
    pub fan_in_timeout: Option<Duration>,
}

/// Tracks the completion status of upstream paths feeding into a deferred node.
#[derive(Debug, Clone)]
pub struct FanInTracker {
    /// Names of all expected upstream paths.
    pub expected_paths: Vec<String>,
    /// Results received so far, keyed by path name.
    pub received: HashMap<String, serde_json::Value>,
}

impl FanInTracker {
    /// Create a new tracker expecting results from the given paths.
    pub fn new(expected_paths: Vec<String>) -> Self {
        Self { expected_paths, received: HashMap::new() }
    }

    /// Record a result from an upstream path.
    pub fn record(&mut self, path_name: &str, value: serde_json::Value) {
        self.received.insert(path_name.to_string(), value);
    }

    /// Check if all expected paths have reported results.
    pub fn is_complete(&self) -> bool {
        self.expected_paths.iter().all(|p| self.received.contains_key(p))
    }

    /// Apply the configured merge strategy to produce the final merged output.
    pub fn merge(&self, strategy: &MergeStrategy) -> serde_json::Value {
        match strategy {
            MergeStrategy::Collect => {
                let values: Vec<serde_json::Value> = self
                    .expected_paths
                    .iter()
                    .filter_map(|p| self.received.get(p).cloned())
                    .collect();
                serde_json::Value::Array(values)
            }
            MergeStrategy::MergeMap => {
                let mut merged = serde_json::Map::new();
                for path in &self.expected_paths {
                    if let Some(serde_json::Value::Object(map)) = self.received.get(path) {
                        for (k, v) in map {
                            merged.insert(k.clone(), v.clone());
                        }
                    }
                }
                serde_json::Value::Object(merged)
            }
        }
    }
}

// ===========================================================================
// Simulated Research Branches
// ===========================================================================

/// Simulates a parallel research branch that queries a specific aspect of
/// "Artificial Intelligence" — representing an LLM-powered research task.
///
/// Each branch has a different simulated latency to demonstrate fan-in
/// timing behavior.
async fn research_history(delay: Duration) -> serde_json::Value {
    let start = Instant::now();
    tokio::time::sleep(delay).await;
    let elapsed = start.elapsed();

    serde_json::json!({
        "branch": "history",
        "topic": "History of Artificial Intelligence",
        "findings": "AI research began in the 1950s with pioneers like Alan Turing, \
                     John McCarthy, and Marvin Minsky. The Dartmouth Conference of 1956 \
                     is widely considered the birth of AI as a field. Early work focused \
                     on symbolic reasoning and expert systems, followed by the neural \
                     network renaissance in the 1980s-90s, and the deep learning \
                     revolution from 2012 onward.",
        "elapsed_ms": elapsed.as_millis()
    })
}

async fn research_technology(delay: Duration) -> serde_json::Value {
    let start = Instant::now();
    tokio::time::sleep(delay).await;
    let elapsed = start.elapsed();

    serde_json::json!({
        "branch": "technology",
        "topic": "Current AI Technologies",
        "findings": "Modern AI is dominated by transformer architectures (GPT, BERT, \
                     Gemini), diffusion models for image generation, and reinforcement \
                     learning from human feedback (RLHF). Key capabilities include \
                     natural language understanding, code generation, multimodal \
                     reasoning, and agentic tool use. Hardware advances (TPUs, H100 \
                     GPUs) enable training at unprecedented scale.",
        "elapsed_ms": elapsed.as_millis()
    })
}

async fn research_economics(delay: Duration) -> serde_json::Value {
    let start = Instant::now();
    tokio::time::sleep(delay).await;
    let elapsed = start.elapsed();

    serde_json::json!({
        "branch": "economics",
        "topic": "Economic Impact of AI",
        "findings": "AI is projected to add $15.7 trillion to the global economy by \
                     2030 (PwC). Key sectors include healthcare (drug discovery, \
                     diagnostics), finance (algorithmic trading, fraud detection), \
                     manufacturing (predictive maintenance), and services (customer \
                     support automation). Labor market impacts include both job \
                     displacement and creation of new roles.",
        "elapsed_ms": elapsed.as_millis()
    })
}

// ===========================================================================
// Deferred Node Execution Logic
// ===========================================================================

/// Execute all research branches in parallel and apply the deferred node's
/// merge strategy, respecting the configured fan-in timeout.
///
/// Returns the merged result and timing metadata.
async fn execute_deferred_node(
    config: &DeferredNodeConfig,
    branch_delays: &[(&str, Duration)],
) -> (serde_json::Value, Duration, Vec<String>) {
    let start = Instant::now();
    let path_names: Vec<String> = branch_delays.iter().map(|(n, _)| n.to_string()).collect();
    let mut tracker = FanInTracker::new(path_names.clone());
    let mut completed_paths: Vec<String> = Vec::new();

    // Spawn all research branches as concurrent tasks
    let mut handles: Vec<(String, tokio::task::JoinHandle<serde_json::Value>)> = Vec::new();

    for (name, delay) in branch_delays {
        let delay = *delay;
        let name = name.to_string();
        let name_clone = name.clone();
        let handle = match name.as_str() {
            "history" => tokio::spawn(async move { research_history(delay).await }),
            "technology" => tokio::spawn(async move { research_technology(delay).await }),
            "economics" => tokio::spawn(async move { research_economics(delay).await }),
            _ => tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                serde_json::json!({"branch": name_clone, "findings": "unknown branch"})
            }),
        };
        handles.push((name, handle));
    }

    // Wait for results with optional timeout
    match config.fan_in_timeout {
        Some(timeout) => {
            // Use tokio::time::timeout to enforce the fan-in deadline
            let deadline = tokio::time::sleep(timeout);
            tokio::pin!(deadline);

            for (name, handle) in handles {
                tokio::select! {
                    result = handle => {
                        if let Ok(value) = result {
                            tracker.record(&name, value);
                            completed_paths.push(name);
                        }
                    }
                    _ = &mut deadline => {
                        // Timeout reached — stop waiting for remaining paths
                        println!("  ⚠ Fan-in timeout reached ({timeout:?}), \
                                  path '{name}' did not complete in time");
                        break;
                    }
                }
            }
        }
        None => {
            // No timeout — wait for all paths
            for (name, handle) in handles {
                if let Ok(value) = handle.await {
                    tracker.record(&name, value);
                    completed_paths.push(name);
                }
            }
        }
    }

    let merged = tracker.merge(&config.merge_strategy);
    let total_elapsed = start.elapsed();

    (merged, total_elapsed, completed_paths)
}

// ===========================================================================
// Helper: require an environment variable or exit with a descriptive message
// ===========================================================================

fn require_env(name: &str) -> anyhow::Result<String> {
    std::env::var(name).map_err(|_| {
        anyhow::anyhow!(
            "Missing required environment variable: {name}\n\
             Set it in your .env file or export it in your shell.\n\
             See .env.example for all required variables."
        )
    })
}

// ===========================================================================
// Helper: classify common LLM errors into actionable categories
// ===========================================================================

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

// ===========================================================================
// Main
// ===========================================================================

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // --- Environment Setup ---
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    println!("╔══════════════════════════════════════════╗");
    println!("║  Deferred Nodes — ADK-Rust v0.8.0        ║");
    println!("╚══════════════════════════════════════════╝\n");

    let api_key = require_env("GOOGLE_API_KEY")?;

    // -----------------------------------------------------------------------
    // Step 1: Configure parallel research branches
    // -----------------------------------------------------------------------

    println!("--- Step 1: Configure Parallel Research Branches ---\n");
    println!("  → Topic: \"Artificial Intelligence\"");
    println!("  → Branch 1: History        (simulated delay: 500ms)");
    println!("  → Branch 2: Technology     (simulated delay: 800ms)");
    println!("  → Branch 3: Economics      (simulated delay: 1000ms)");
    println!("  ✓ API key loaded ({} chars)", api_key.len());
    println!("  ✓ Three parallel upstream paths configured");

    // -----------------------------------------------------------------------
    // Step 2: Execute with MergeStrategy::Collect
    // -----------------------------------------------------------------------

    println!("\n--- Step 2: MergeStrategy::Collect ---\n");
    println!("  → Gathering all upstream outputs into a Vec<Value>");

    let collect_config = DeferredNodeConfig {
        merge_strategy: MergeStrategy::Collect,
        fan_in_timeout: None, // Wait for all paths
    };

    let branch_delays = vec![
        ("history", Duration::from_millis(500)),
        ("technology", Duration::from_millis(800)),
        ("economics", Duration::from_millis(1000)),
    ];

    let (collect_result, collect_elapsed, collect_paths) =
        execute_deferred_node(&collect_config, &branch_delays).await;

    println!("  ✓ All {} paths completed in {:?}", collect_paths.len(), collect_elapsed);
    println!("  ✓ Completed paths: {:?}", collect_paths);
    println!("  ✓ Merge strategy: {}", collect_config.merge_strategy);
    println!(
        "  → Result type: Array with {} elements",
        collect_result.as_array().map_or(0, |a| a.len())
    );

    if let Some(arr) = collect_result.as_array() {
        for (i, item) in arr.iter().enumerate() {
            if let Some(branch) = item.get("branch").and_then(|b| b.as_str()) {
                let elapsed_ms = item.get("elapsed_ms").and_then(|e| e.as_u64()).unwrap_or(0);
                println!("    [{i}] branch=\"{branch}\", completed_in={elapsed_ms}ms");
            }
        }
    }

    // -----------------------------------------------------------------------
    // Step 3: Execute with MergeStrategy::MergeMap
    // -----------------------------------------------------------------------

    println!("\n--- Step 3: MergeStrategy::MergeMap ---\n");
    println!("  → Merging upstream state maps into a single map");

    let merge_config =
        DeferredNodeConfig { merge_strategy: MergeStrategy::MergeMap, fan_in_timeout: None };

    let branch_delays = vec![
        ("history", Duration::from_millis(500)),
        ("technology", Duration::from_millis(800)),
        ("economics", Duration::from_millis(1000)),
    ];

    let (merge_result, merge_elapsed, merge_paths) =
        execute_deferred_node(&merge_config, &branch_delays).await;

    println!("  ✓ All {} paths completed in {:?}", merge_paths.len(), merge_elapsed);
    println!("  ✓ Merge strategy: {}", merge_config.merge_strategy);

    if let Some(obj) = merge_result.as_object() {
        println!("  → Result type: Object with {} keys", obj.len());
        for key in obj.keys() {
            let value_preview = obj
                .get(key)
                .map(|v| {
                    let s = v.to_string();
                    if s.len() > 60 {
                        let mut end = 60;
                        while end > 0 && !s.is_char_boundary(end) {
                            end -= 1;
                        }
                        format!("{}...", &s[..end])
                    } else {
                        s
                    }
                })
                .unwrap_or_default();
            println!("    \"{key}\": {value_preview}");
        }
    }

    // -----------------------------------------------------------------------
    // Step 4: Demonstrate fan_in_timeout
    // -----------------------------------------------------------------------

    println!("\n--- Step 4: Fan-In Timeout ---\n");
    println!("  → Configuring fan_in_timeout = 1200ms");
    println!("  → Branch 1 (history):    delay = 500ms  (will complete)");
    println!("  → Branch 2 (technology): delay = 800ms  (will complete)");
    println!("  → Branch 3 (economics):  delay = 3000ms (will TIMEOUT)");

    let timeout_config = DeferredNodeConfig {
        merge_strategy: MergeStrategy::Collect,
        fan_in_timeout: Some(Duration::from_millis(1200)),
    };

    // One branch is intentionally slow (3s) to trigger the timeout
    let branch_delays_with_slow = vec![
        ("history", Duration::from_millis(500)),
        ("technology", Duration::from_millis(800)),
        ("economics", Duration::from_millis(3000)), // Slow — will timeout
    ];

    let (timeout_result, timeout_elapsed, timeout_paths) =
        execute_deferred_node(&timeout_config, &branch_delays_with_slow).await;

    println!("  ✓ Deferred node completed in {:?}", timeout_elapsed);
    println!("  ✓ Paths that completed before timeout: {:?}", timeout_paths);
    println!(
        "  → Partial result: Array with {} elements (out of 3 expected)",
        timeout_result.as_array().map_or(0, |a| a.len())
    );

    if let Some(arr) = timeout_result.as_array() {
        for (i, item) in arr.iter().enumerate() {
            if let Some(branch) = item.get("branch").and_then(|b| b.as_str()) {
                let elapsed_ms = item.get("elapsed_ms").and_then(|e| e.as_u64()).unwrap_or(0);
                println!("    [{i}] branch=\"{branch}\", completed_in={elapsed_ms}ms");
            }
        }
    }

    // Check which paths were missed
    let all_expected = ["history", "technology", "economics"];
    let missed: Vec<&str> =
        all_expected.iter().filter(|p| !timeout_paths.contains(&p.to_string())).copied().collect();
    if !missed.is_empty() {
        println!("  ⚠ Paths excluded due to timeout: {:?}", missed);
    }

    // -----------------------------------------------------------------------
    // Summary
    // -----------------------------------------------------------------------

    println!("\n--- Summary ---\n");
    println!("  Parallel branches configured:  3 (history, technology, economics)");
    println!(
        "  MergeStrategy::Collect:        ✓ gathered {} items into Vec",
        collect_result.as_array().map_or(0, |a| a.len())
    );
    println!(
        "  MergeStrategy::MergeMap:       ✓ merged into map with {} keys",
        merge_result.as_object().map_or(0, |o| o.len())
    );
    println!(
        "  Fan-in timeout (1200ms):       ✓ {}/{} paths completed before deadline",
        timeout_paths.len(),
        all_expected.len()
    );
    println!("  Total Collect elapsed:         {:?}", collect_elapsed);
    println!("  Total MergeMap elapsed:        {:?}", merge_elapsed);
    println!("  Total Timeout elapsed:         {:?}", timeout_elapsed);
    println!("\n✅ Deferred Nodes example completed successfully.");

    Ok(())
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_require_env_missing_variable() {
        let result = require_env("__ADK_TEST_NONEXISTENT_VAR_12345__");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("__ADK_TEST_NONEXISTENT_VAR_12345__"));
        assert!(err_msg.contains(".env.example"));
    }

    #[test]
    fn test_require_env_present_variable() {
        unsafe { std::env::set_var("__ADK_TEST_PRESENT_VAR__", "test_value") };
        let result = require_env("__ADK_TEST_PRESENT_VAR__");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test_value");
        unsafe { std::env::remove_var("__ADK_TEST_PRESENT_VAR__") };
    }

    #[test]
    fn test_classify_llm_error_auth() {
        let err = anyhow::anyhow!("HTTP 401 Unauthorized");
        assert!(classify_llm_error(&err).contains("Authentication"));
    }

    #[test]
    fn test_classify_llm_error_rate_limit() {
        let err = anyhow::anyhow!("HTTP 429 rate limit exceeded");
        assert!(classify_llm_error(&err).contains("Rate limited"));
    }

    #[test]
    fn test_classify_llm_error_context_length() {
        let err = anyhow::anyhow!("token limit exceeded: context length too large");
        assert!(classify_llm_error(&err).contains("Context too large"));
    }

    #[test]
    fn test_classify_llm_error_unknown() {
        let err = anyhow::anyhow!("some random network error");
        assert!(classify_llm_error(&err).contains("Unexpected error"));
    }

    #[test]
    fn test_merge_strategy_display() {
        assert_eq!(format!("{}", MergeStrategy::Collect), "Collect");
        assert_eq!(format!("{}", MergeStrategy::MergeMap), "MergeMap");
    }

    #[test]
    fn test_fan_in_tracker_new() {
        let tracker = FanInTracker::new(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        assert_eq!(tracker.expected_paths.len(), 3);
        assert!(tracker.received.is_empty());
        assert!(!tracker.is_complete());
    }

    #[test]
    fn test_fan_in_tracker_record_and_complete() {
        let mut tracker = FanInTracker::new(vec!["a".to_string(), "b".to_string()]);

        tracker.record("a", serde_json::json!({"result": "alpha"}));
        assert!(!tracker.is_complete());

        tracker.record("b", serde_json::json!({"result": "beta"}));
        assert!(tracker.is_complete());
    }

    #[test]
    fn test_merge_collect() {
        let mut tracker = FanInTracker::new(vec!["x".to_string(), "y".to_string()]);
        tracker.record("x", serde_json::json!({"val": 1}));
        tracker.record("y", serde_json::json!({"val": 2}));

        let result = tracker.merge(&MergeStrategy::Collect);
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0], serde_json::json!({"val": 1}));
        assert_eq!(arr[1], serde_json::json!({"val": 2}));
    }

    #[test]
    fn test_merge_merge_map() {
        let mut tracker = FanInTracker::new(vec!["a".to_string(), "b".to_string()]);
        tracker.record("a", serde_json::json!({"key1": "val1", "key2": "val2"}));
        tracker.record("b", serde_json::json!({"key3": "val3", "key4": "val4"}));

        let result = tracker.merge(&MergeStrategy::MergeMap);
        let obj = result.as_object().unwrap();
        assert_eq!(obj.len(), 4);
        assert_eq!(obj.get("key1").unwrap(), "val1");
        assert_eq!(obj.get("key2").unwrap(), "val2");
        assert_eq!(obj.get("key3").unwrap(), "val3");
        assert_eq!(obj.get("key4").unwrap(), "val4");
    }

    #[test]
    fn test_merge_merge_map_overwrites_duplicates() {
        let mut tracker = FanInTracker::new(vec!["first".to_string(), "second".to_string()]);
        tracker.record("first", serde_json::json!({"shared": "from_first"}));
        tracker.record("second", serde_json::json!({"shared": "from_second"}));

        let result = tracker.merge(&MergeStrategy::MergeMap);
        let obj = result.as_object().unwrap();
        // "second" comes after "first" in expected_paths, so it overwrites
        assert_eq!(obj.get("shared").unwrap(), "from_second");
    }

    #[test]
    fn test_merge_collect_partial() {
        // When not all paths have reported, Collect only includes received ones
        let mut tracker =
            FanInTracker::new(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        tracker.record("a", serde_json::json!("result_a"));
        tracker.record("c", serde_json::json!("result_c"));
        // "b" is missing

        let result = tracker.merge(&MergeStrategy::Collect);
        let arr = result.as_array().unwrap();
        // Only "a" and "c" are present (in expected_paths order)
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0], serde_json::json!("result_a"));
        assert_eq!(arr[1], serde_json::json!("result_c"));
    }

    #[tokio::test]
    async fn test_execute_deferred_node_collect_all_complete() {
        let config =
            DeferredNodeConfig { merge_strategy: MergeStrategy::Collect, fan_in_timeout: None };
        let delays = vec![
            ("history", Duration::from_millis(50)),
            ("technology", Duration::from_millis(100)),
            ("economics", Duration::from_millis(150)),
        ];

        let (result, _elapsed, paths) = execute_deferred_node(&config, &delays).await;

        assert_eq!(paths.len(), 3);
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 3);
    }

    #[tokio::test]
    async fn test_execute_deferred_node_timeout_partial() {
        let config = DeferredNodeConfig {
            merge_strategy: MergeStrategy::Collect,
            fan_in_timeout: Some(Duration::from_millis(120)),
        };
        let delays = vec![
            ("history", Duration::from_millis(50)),
            ("technology", Duration::from_millis(80)),
            ("economics", Duration::from_millis(500)), // Will timeout
        ];

        let (result, _elapsed, paths) = execute_deferred_node(&config, &delays).await;

        // At least the first two should complete before the 120ms timeout
        assert!(paths.len() >= 2);
        let arr = result.as_array().unwrap();
        assert!(arr.len() >= 2);
    }
}
