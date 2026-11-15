//! # Node Timeouts Example
//!
//! Demonstrates ADK-Rust's node-level timeout policies with wall-clock timeouts,
//! idle timeouts, and recovery actions (`OnTimeout::Fail`, `OnTimeout::Retry`,
//! `OnTimeout::Skip`) in graph-based workflows.
//!
//! ## What This Shows
//!
//! - Configuring `TimeoutPolicy` with `run_timeout` (wall-clock) and `idle_timeout`
//! - Using `OnTimeout::Fail` to abort a node that exceeds its time budget
//! - Using `OnTimeout::Retry { max_attempts }` to automatically retry timed-out nodes
//! - Using `OnTimeout::Skip` to gracefully skip optional nodes that stall
//! - Using `report_progress()` to keep a node alive during long operations
//! - Printing timeout events and recovery actions for observability
//!
//! ## Prerequisites
//!
//! - `GOOGLE_API_KEY` environment variable set (for the Gemini LLM provider)
//!
//! ## Run
//!
//! ```bash
//! cargo run --manifest-path examples/node_timeouts/Cargo.toml
//! ```

use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing_subscriber::EnvFilter;

// ===========================================================================
// Simulated Timeout Policy Types (demonstrating the planned ADK-Rust API)
// ===========================================================================

/// Recovery action when a node exceeds its timeout budget.
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum OnTimeout {
    /// Abort the node immediately — the graph fails at this point.
    Fail,
    /// Retry the node up to `max_attempts` times before failing.
    Retry { max_attempts: u32 },
    /// Skip the node and continue the graph with a default/empty output.
    Skip,
}

/// Timeout configuration for a single graph node.
#[derive(Debug, Clone)]
struct TimeoutPolicy {
    /// Maximum wall-clock duration for the node's execution.
    run_timeout: Option<Duration>,
    /// Maximum duration without a progress report before the node is considered stalled.
    idle_timeout: Option<Duration>,
    /// What to do when a timeout fires.
    on_timeout: OnTimeout,
}

/// A progress handle that nodes use to report liveness during long operations.
/// Sending a message resets the idle timeout counter.
#[derive(Clone)]
struct ProgressHandle {
    sender: mpsc::Sender<()>,
}

impl ProgressHandle {
    /// Report progress to the timeout monitor, resetting the idle timer.
    async fn report_progress(&self) {
        let _ = self.sender.send(()).await;
    }
}

/// Outcome of executing a node with timeout enforcement.
#[derive(Debug)]
enum NodeOutcome {
    /// Node completed successfully within its time budget.
    Success { result: String, elapsed: Duration },
    /// Node was aborted because it exceeded its wall-clock or idle timeout.
    TimedOut { elapsed: Duration, action: String },
    /// Node was skipped after timeout (OnTimeout::Skip).
    Skipped { reason: String, elapsed: Duration },
}

/// A simulated graph node with a name, timeout policy, and async work function.
struct GraphNode {
    name: String,
    policy: TimeoutPolicy,
}

// ===========================================================================
// Timeout execution engine (simulates adk-graph's execute_with_timeout)
// ===========================================================================

/// Execute a node's work function with wall-clock timeout enforcement.
/// Returns the outcome including timing and recovery action taken.
async fn execute_with_wall_clock_timeout<F, Fut>(
    node: &GraphNode,
    work: F,
) -> NodeOutcome
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = String>,
{
    let run_limit = node
        .policy
        .run_timeout
        .unwrap_or(Duration::from_secs(300));

    let start = Instant::now();
    match timeout(run_limit, work()).await {
        Ok(result) => NodeOutcome::Success {
            result,
            elapsed: start.elapsed(),
        },
        Err(_) => {
            let elapsed = start.elapsed();
            match &node.policy.on_timeout {
                OnTimeout::Fail => NodeOutcome::TimedOut {
                    elapsed,
                    action: "FAIL — node aborted, graph halted".to_string(),
                },
                OnTimeout::Skip => NodeOutcome::Skipped {
                    reason: format!("wall-clock timeout after {elapsed:.2?}"),
                    elapsed,
                },
                OnTimeout::Retry { .. } => NodeOutcome::TimedOut {
                    elapsed,
                    action: "RETRY — will attempt again".to_string(),
                },
            }
        }
    }
}

/// Execute a node's work function with idle timeout enforcement.
/// The node must periodically call `progress_handle.report_progress()` to stay alive.
/// If no progress is reported within `idle_timeout`, the node is terminated.
async fn execute_with_idle_timeout<F, Fut>(
    node: &GraphNode,
    work: F,
) -> NodeOutcome
where
    F: FnOnce(ProgressHandle) -> Fut,
    Fut: std::future::Future<Output = String> + Send + 'static,
{
    let idle_limit = node
        .policy
        .idle_timeout
        .unwrap_or(Duration::from_secs(300));

    let (tx, mut rx) = mpsc::channel::<()>(16);
    let handle = ProgressHandle { sender: tx };

    let start = Instant::now();

    // Spawn the work in a task so we can monitor progress independently
    let work_handle = tokio::spawn(work(handle));

    // Monitor: wait for progress reports; if none arrive within idle_limit, cancel
    loop {
        match timeout(idle_limit, rx.recv()).await {
            Ok(Some(())) => {
                // Progress reported — reset idle timer (loop continues)
                continue;
            }
            Ok(None) => {
                // Channel closed — work completed
                break;
            }
            Err(_) => {
                // Idle timeout fired — no progress within the limit
                work_handle.abort();
                let elapsed = start.elapsed();
                return match &node.policy.on_timeout {
                    OnTimeout::Fail => NodeOutcome::TimedOut {
                        elapsed,
                        action: "FAIL — idle timeout, no progress reported".to_string(),
                    },
                    OnTimeout::Skip => NodeOutcome::Skipped {
                        reason: format!("idle timeout after {elapsed:.2?} (no progress)"),
                        elapsed,
                    },
                    OnTimeout::Retry { .. } => NodeOutcome::TimedOut {
                        elapsed,
                        action: "RETRY — idle timeout".to_string(),
                    },
                };
            }
        }
    }

    // Work completed normally
    match work_handle.await {
        Ok(result) => NodeOutcome::Success {
            result,
            elapsed: start.elapsed(),
        },
        Err(_) => NodeOutcome::TimedOut {
            elapsed: start.elapsed(),
            action: "FAIL — task panicked".to_string(),
        },
    }
}

/// Execute a node with retry logic for wall-clock timeouts.
/// Retries up to `max_attempts` times, with each attempt getting progressively
/// faster (simulating a node that adapts its work on retry).
async fn execute_with_retry<F, Fut>(
    node: &GraphNode,
    max_attempts: u32,
    mut work_factory: F,
) -> (NodeOutcome, u32)
where
    F: FnMut(u32) -> Fut,
    Fut: std::future::Future<Output = String>,
{
    let run_limit = node
        .policy
        .run_timeout
        .unwrap_or(Duration::from_secs(300));

    let overall_start = Instant::now();

    for attempt in 1..=max_attempts {
        let start = Instant::now();
        match timeout(run_limit, work_factory(attempt)).await {
            Ok(result) => {
                return (
                    NodeOutcome::Success {
                        result,
                        elapsed: start.elapsed(),
                    },
                    attempt,
                );
            }
            Err(_) => {
                if attempt == max_attempts {
                    return (
                        NodeOutcome::TimedOut {
                            elapsed: overall_start.elapsed(),
                            action: format!(
                                "FAIL — exhausted all {max_attempts} retry attempts"
                            ),
                        },
                        attempt,
                    );
                }
                // Will retry on next iteration
            }
        }
    }
    unreachable!()
}

// ===========================================================================
// Helper: require an environment variable or exit with a descriptive message
// ===========================================================================

/// Loads a required environment variable, returning an actionable error if missing.
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

/// Inspects an error message and returns a human-readable classification.
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
// Output formatting helpers
// ===========================================================================

fn print_banner() {
    println!("╔══════════════════════════════════════════╗");
    println!("║  Node Timeouts — ADK-Rust v0.8.0         ║");
    println!("╚══════════════════════════════════════════╝\n");
}

fn print_step(n: u32, description: &str) {
    println!("--- Step {n}: {description} ---\n");
}

fn print_success(msg: &str) {
    println!("  ✓ {msg}");
}

fn print_progress(msg: &str) {
    println!("  → {msg}");
}

fn print_warning(msg: &str) {
    println!("  ⚠ {msg}");
}

fn print_summary(lines: &[&str]) {
    println!("\n--- Summary ---\n");
    for line in lines {
        println!("  {line}");
    }
    println!("\n✅ Example completed successfully.");
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

    print_banner();

    let api_key = require_env("GOOGLE_API_KEY")?;
    print_success(&format!("GOOGLE_API_KEY loaded ({} chars)", api_key.len()));
    print_progress("Using model: gemini-2.5-flash (timeout demo uses simulated nodes)\n");

    // ===================================================================
    // Step 1: Configure Timeout Policies for Three Nodes
    // ===================================================================

    print_step(1, "Configure Timeout Policies");

    let fail_node = GraphNode {
        name: "fast_research".to_string(),
        policy: TimeoutPolicy {
            run_timeout: Some(Duration::from_secs(2)),
            idle_timeout: None,
            on_timeout: OnTimeout::Fail,
        },
    };
    print_success(&format!(
        "Node '{}': run_timeout=2s, on_timeout=Fail",
        fail_node.name
    ));

    let retry_node = GraphNode {
        name: "retry_analysis".to_string(),
        policy: TimeoutPolicy {
            run_timeout: Some(Duration::from_secs(3)),
            idle_timeout: None,
            on_timeout: OnTimeout::Retry { max_attempts: 3 },
        },
    };
    print_success(&format!(
        "Node '{}': run_timeout=3s, on_timeout=Retry(max_attempts=3)",
        retry_node.name
    ));

    let skip_node = GraphNode {
        name: "optional_enrichment".to_string(),
        policy: TimeoutPolicy {
            run_timeout: None,
            idle_timeout: Some(Duration::from_secs(2)),
            on_timeout: OnTimeout::Skip,
        },
    };
    print_success(&format!(
        "Node '{}': idle_timeout=2s, on_timeout=Skip",
        skip_node.name
    ));

    println!();

    // ===================================================================
    // Step 2: Execute Node with OnTimeout::Fail (wall-clock timeout)
    // ===================================================================

    print_step(2, "Execute 'fast_research' — Wall-Clock Timeout → Fail");
    print_progress("Node intentionally sleeps 4s with a 2s wall-clock limit...");

    let outcome = execute_with_wall_clock_timeout(&fail_node, || async {
        // Simulate expensive research that takes 4 seconds (exceeds 2s limit)
        tokio::time::sleep(Duration::from_secs(4)).await;
        "Research complete: found 15 relevant papers".to_string()
    })
    .await;

    match &outcome {
        NodeOutcome::TimedOut { elapsed, action } => {
            print_warning(&format!(
                "TIMEOUT on '{}' after {elapsed:.2?}",
                fail_node.name
            ));
            print_warning(&format!("Recovery: {action}"));
        }
        NodeOutcome::Success { result, elapsed } => {
            print_success(&format!(
                "'{}' completed in {elapsed:.2?}: {result}",
                fail_node.name
            ));
        }
        NodeOutcome::Skipped { reason, .. } => {
            print_progress(&format!("'{}' skipped: {reason}", fail_node.name));
        }
    }

    println!();

    // ===================================================================
    // Step 3: Execute Node with OnTimeout::Retry (wall-clock timeout)
    // ===================================================================

    print_step(
        3,
        "Execute 'retry_analysis' — Wall-Clock Timeout → Retry (max 3 attempts)",
    );
    print_progress("Node adapts work duration on each retry attempt...");
    print_progress("  Attempt 1: sleeps 5s (exceeds 3s limit → timeout)");
    print_progress("  Attempt 2: sleeps 4s (exceeds 3s limit → timeout)");
    print_progress("  Attempt 3: sleeps 2s (within 3s limit → success!)");

    let (outcome, attempts) = execute_with_retry(
        &retry_node,
        3,
        |attempt| async move {
            // Simulate progressively faster work on each retry:
            // Attempt 1: 5s (too slow), Attempt 2: 4s (too slow), Attempt 3: 2s (fast enough)
            let sleep_duration = match attempt {
                1 => Duration::from_millis(5000),
                2 => Duration::from_millis(4000),
                _ => Duration::from_millis(2000),
            };
            tokio::time::sleep(sleep_duration).await;
            format!("Analysis complete on attempt {attempt}: sentiment=positive, confidence=0.87")
        },
    )
    .await;

    match &outcome {
        NodeOutcome::Success { result, elapsed } => {
            print_success(&format!(
                "'{}' succeeded on attempt {attempts} in {elapsed:.2?}",
                retry_node.name
            ));
            print_success(&format!("Result: {result}"));
        }
        NodeOutcome::TimedOut { elapsed, action } => {
            print_warning(&format!(
                "'{}' failed after {attempts} attempts ({elapsed:.2?})",
                retry_node.name
            ));
            print_warning(&format!("Recovery: {action}"));
        }
        _ => {}
    }

    println!();

    // ===================================================================
    // Step 4: Execute Node with OnTimeout::Skip (idle timeout — stalls)
    // ===================================================================

    print_step(
        4,
        "Execute 'optional_enrichment' — Idle Timeout → Skip (node stalls)",
    );
    print_progress("Node reports progress once, then stops — idle timeout fires after 2s...");

    let outcome = execute_with_idle_timeout(&skip_node, |handle| async move {
        // Report initial progress
        handle.report_progress().await;

        // Do some initial work
        tokio::time::sleep(Duration::from_millis(500)).await;
        handle.report_progress().await;

        // Now the node "stalls" — stops reporting progress for longer than idle_timeout
        // This simulates a node waiting on an external resource that never responds
        tokio::time::sleep(Duration::from_secs(10)).await;

        "Enrichment data: added 5 metadata fields".to_string()
    })
    .await;

    match &outcome {
        NodeOutcome::Skipped { reason, elapsed } => {
            print_warning(&format!(
                "'{}' idle timeout after {elapsed:.2?}",
                skip_node.name
            ));
            print_progress(&format!("Recovery: SKIP — {reason}"));
            print_progress("Graph continues without enrichment data (graceful degradation)");
        }
        NodeOutcome::Success { result, elapsed } => {
            print_success(&format!(
                "'{}' completed in {elapsed:.2?}: {result}",
                skip_node.name
            ));
        }
        NodeOutcome::TimedOut { elapsed, action } => {
            print_warning(&format!(
                "'{}' timed out after {elapsed:.2?}: {action}",
                skip_node.name
            ));
        }
    }

    println!();

    // ===================================================================
    // Step 5: Demonstrate report_progress() keeping a node alive
    // ===================================================================

    print_step(
        5,
        "Demonstrate report_progress() — Node Avoids Idle Timeout",
    );

    let progress_node = GraphNode {
        name: "active_enrichment".to_string(),
        policy: TimeoutPolicy {
            run_timeout: None,
            idle_timeout: Some(Duration::from_secs(2)),
            on_timeout: OnTimeout::Fail,
        },
    };
    print_progress(&format!(
        "Node '{}': idle_timeout=2s, but reports progress every 1s",
        progress_node.name
    ));
    print_progress("Total work takes ~4s — would timeout without progress reports...");

    let outcome = execute_with_idle_timeout(&progress_node, |handle| async move {
        let mut collected = Vec::new();

        // Simulate 4 phases of work, each ~1s, reporting progress between them
        for phase in 1..=4 {
            tokio::time::sleep(Duration::from_millis(900)).await;
            handle.report_progress().await;
            collected.push(format!("phase_{phase}"));
            // This would be printed in a real scenario for observability
        }

        format!("Enrichment complete: processed {} phases", collected.len())
    })
    .await;

    match &outcome {
        NodeOutcome::Success { result, elapsed } => {
            print_success(&format!(
                "'{}' completed in {elapsed:.2?} (survived 4 idle windows!)",
                progress_node.name
            ));
            print_success(&format!("Result: {result}"));
        }
        NodeOutcome::TimedOut { elapsed, action } => {
            print_warning(&format!(
                "'{}' unexpectedly timed out after {elapsed:.2?}: {action}",
                progress_node.name
            ));
        }
        NodeOutcome::Skipped { reason, .. } => {
            print_progress(&format!("'{}' skipped: {reason}", progress_node.name));
        }
    }

    println!();

    // ===================================================================
    // Summary
    // ===================================================================

    print_summary(&[
        "Timeout policies configured: 4 nodes with different recovery actions",
        "",
        "Node 'fast_research':        run_timeout=2s, OnTimeout::Fail",
        "  → Timed out and aborted (intentionally exceeded limit)",
        "",
        "Node 'retry_analysis':       run_timeout=3s, OnTimeout::Retry(3)",
        "  → Timed out twice, succeeded on attempt 3 (adapted work)",
        "",
        "Node 'optional_enrichment':  idle_timeout=2s, OnTimeout::Skip",
        "  → Stalled (no progress), skipped gracefully",
        "",
        "Node 'active_enrichment':    idle_timeout=2s, report_progress()",
        "  → Reported progress every ~1s, completed 4s of work without timeout",
        "",
        "Key takeaways:",
        "  • run_timeout enforces a hard wall-clock deadline",
        "  • idle_timeout detects stalled nodes that stop making progress",
        "  • report_progress() resets the idle timer — keeps active nodes alive",
        "  • OnTimeout::Retry enables automatic recovery for transient slowness",
        "  • OnTimeout::Skip enables graceful degradation for optional work",
    ]);

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
        let result = require_env("DEFINITELY_NOT_SET_XYZ_12345");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("DEFINITELY_NOT_SET_XYZ_12345"),
            "Error should contain the variable name"
        );
        assert!(
            err_msg.contains(".env.example"),
            "Error should reference .env.example"
        );
    }

    #[test]
    fn test_require_env_present_variable() {
        unsafe { std::env::set_var("TEST_NODE_TIMEOUTS_VAR", "test_value") };
        let result = require_env("TEST_NODE_TIMEOUTS_VAR");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test_value");
        unsafe { std::env::remove_var("TEST_NODE_TIMEOUTS_VAR") };
    }

    #[test]
    fn test_classify_llm_error_auth() {
        let err = anyhow::anyhow!("HTTP 401 Unauthorized");
        assert!(classify_llm_error(&err).contains("Authentication failed"));
    }

    #[test]
    fn test_classify_llm_error_rate_limit() {
        let err = anyhow::anyhow!("429 Too Many Requests - rate limit exceeded");
        assert!(classify_llm_error(&err).contains("Rate limited"));
    }

    #[test]
    fn test_classify_llm_error_token_limit() {
        let err = anyhow::anyhow!("token limit exceeded: context length too large");
        assert!(classify_llm_error(&err).contains("Context too large"));
    }

    #[test]
    fn test_classify_llm_error_unknown() {
        let err = anyhow::anyhow!("some random network failure");
        assert!(classify_llm_error(&err).contains("Unexpected error"));
    }

    #[tokio::test]
    async fn test_wall_clock_timeout_fires() {
        let node = GraphNode {
            name: "test_node".to_string(),
            policy: TimeoutPolicy {
                run_timeout: Some(Duration::from_millis(100)),
                idle_timeout: None,
                on_timeout: OnTimeout::Fail,
            },
        };

        let outcome = execute_with_wall_clock_timeout(&node, || async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            "should not reach here".to_string()
        })
        .await;

        assert!(matches!(outcome, NodeOutcome::TimedOut { .. }));
    }

    #[tokio::test]
    async fn test_wall_clock_timeout_success() {
        let node = GraphNode {
            name: "test_node".to_string(),
            policy: TimeoutPolicy {
                run_timeout: Some(Duration::from_secs(5)),
                idle_timeout: None,
                on_timeout: OnTimeout::Fail,
            },
        };

        let outcome = execute_with_wall_clock_timeout(&node, || async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            "done".to_string()
        })
        .await;

        assert!(matches!(outcome, NodeOutcome::Success { .. }));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_idle_timeout_fires_when_no_progress() {
        let node = GraphNode {
            name: "stalling_node".to_string(),
            policy: TimeoutPolicy {
                run_timeout: None,
                idle_timeout: Some(Duration::from_millis(200)),
                on_timeout: OnTimeout::Skip,
            },
        };

        let outcome = execute_with_idle_timeout(&node, |handle| async move {
            // Keep the handle alive but never call report_progress()
            // The idle timeout should fire after 200ms of silence
            let _keep_alive = handle;
            tokio::time::sleep(Duration::from_secs(10)).await;
            "should not reach here".to_string()
        })
        .await;

        assert!(
            matches!(outcome, NodeOutcome::Skipped { .. }),
            "Expected Skipped, got {outcome:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_idle_timeout_avoided_with_progress() {
        let node = GraphNode {
            name: "active_node".to_string(),
            policy: TimeoutPolicy {
                run_timeout: None,
                idle_timeout: Some(Duration::from_millis(300)),
                on_timeout: OnTimeout::Fail,
            },
        };

        let outcome = execute_with_idle_timeout(&node, |handle| async move {
            for _ in 0..3 {
                tokio::time::sleep(Duration::from_millis(200)).await;
                handle.report_progress().await;
            }
            "completed with progress".to_string()
        })
        .await;

        assert!(matches!(outcome, NodeOutcome::Success { .. }));
        if let NodeOutcome::Success { result, .. } = outcome {
            assert_eq!(result, "completed with progress");
        }
    }

    #[tokio::test]
    async fn test_retry_succeeds_on_later_attempt() {
        let node = GraphNode {
            name: "retry_node".to_string(),
            policy: TimeoutPolicy {
                run_timeout: Some(Duration::from_millis(150)),
                idle_timeout: None,
                on_timeout: OnTimeout::Retry { max_attempts: 3 },
            },
        };

        let (outcome, attempts) = execute_with_retry(&node, 3, |attempt| async move {
            let delay = match attempt {
                1 => Duration::from_millis(500), // too slow
                2 => Duration::from_millis(500), // too slow
                _ => Duration::from_millis(50),  // fast enough
            };
            tokio::time::sleep(delay).await;
            format!("done on attempt {attempt}")
        })
        .await;

        assert_eq!(attempts, 3);
        assert!(matches!(outcome, NodeOutcome::Success { .. }));
    }
}
