//! # Tool Concurrency Example
//!
//! Demonstrates ADK-Rust's per-tool concurrency limits with backpressure policies,
//! showing how to throttle expensive tools independently while allowing cheap tools
//! to run at higher concurrency.
//!
//! ## What This Shows
//!
//! - Configuring `RunConfig` with `tool_concurrency_overrides` for per-tool limits
//! - `BackpressurePolicy::Queue` behavior: queued execution when limits are exceeded
//! - `BackpressurePolicy::Fail` behavior: immediate rejection on limit exceeded
//! - Timing information showing the effect of concurrency limits on execution duration
//! - A realistic agent scenario with expensive (web scraper) and cheap (calculator) tools
//!
//! ## Prerequisites
//!
//! - `GOOGLE_API_KEY` environment variable set (for the Gemini LLM provider)
//!
//! ## Run
//!
//! ```bash
//! cargo run --manifest-path examples/tool_concurrency/Cargo.toml
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::join_all;
use tokio::sync::Semaphore;
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
// Backpressure Policy
// ---------------------------------------------------------------------------

/// Defines behavior when a tool's concurrency limit is reached.
///
/// This simulates the ADK-Rust `BackpressurePolicy` enum that controls
/// how the runtime handles excess tool invocations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackpressurePolicy {
    /// Queue the call and wait until a slot becomes available.
    Queue,
    /// Immediately reject the call with an error.
    Fail,
}

impl std::fmt::Display for BackpressurePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackpressurePolicy::Queue => write!(f, "Queue"),
            BackpressurePolicy::Fail => write!(f, "Fail"),
        }
    }
}

// ---------------------------------------------------------------------------
// Tool Concurrency Configuration
// ---------------------------------------------------------------------------

/// Per-tool concurrency configuration simulating `ToolConcurrencyConfig`.
///
/// Maps tool names to their maximum concurrent execution slots and defines
/// the backpressure policy when limits are exceeded.
struct ToolConcurrencyConfig {
    /// Per-tool concurrency limits (tool_name → max_concurrent).
    per_tool: HashMap<String, usize>,
    /// What to do when a tool's limit is reached.
    backpressure: BackpressurePolicy,
}

/// Simulates `RunConfig` with tool concurrency settings.
#[allow(dead_code)]
struct RunConfig {
    tool_concurrency: ToolConcurrencyConfig,
    model: String,
}

// ---------------------------------------------------------------------------
// Concurrency Limiter
// ---------------------------------------------------------------------------

/// A per-tool concurrency limiter using tokio Semaphores.
///
/// Each tool gets its own semaphore with a capacity equal to its configured
/// concurrency limit. The limiter enforces the configured `BackpressurePolicy`.
struct ConcurrencyLimiter {
    semaphores: HashMap<String, Arc<Semaphore>>,
    policy: BackpressurePolicy,
}

impl ConcurrencyLimiter {
    /// Create a new limiter from the concurrency configuration.
    fn new(config: &ToolConcurrencyConfig) -> Self {
        let semaphores = config
            .per_tool
            .iter()
            .map(|(name, &limit)| (name.clone(), Arc::new(Semaphore::new(limit))))
            .collect();
        Self {
            semaphores,
            policy: config.backpressure,
        }
    }

    /// Execute a tool call respecting the concurrency limit.
    ///
    /// Under `Queue` policy, waits for a permit. Under `Fail` policy,
    /// returns an error immediately if no permit is available.
    async fn execute<F, T>(
        &self,
        tool_name: &str,
        call_id: usize,
        func: F,
    ) -> Result<T, String>
    where
        F: std::future::Future<Output = T>,
    {
        let semaphore = self
            .semaphores
            .get(tool_name)
            .ok_or_else(|| format!("No concurrency limit configured for tool: {tool_name}"))?;

        match self.policy {
            BackpressurePolicy::Queue => {
                // Wait for a permit (queued execution)
                let _permit = semaphore.acquire().await.map_err(|e| {
                    format!("Semaphore closed for {tool_name} call #{call_id}: {e}")
                })?;
                Ok(func.await)
            }
            BackpressurePolicy::Fail => {
                // Try to acquire immediately; fail if unavailable
                match semaphore.try_acquire() {
                    Ok(_permit) => Ok(func.await),
                    Err(_) => Err(format!(
                        "Concurrency limit exceeded for tool '{tool_name}' (call #{call_id}): \
                         immediate rejection under Fail policy"
                    )),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Simulated Tools
// ---------------------------------------------------------------------------

/// Simulates an expensive "web_scraper" tool with a 2-second artificial delay.
///
/// In a real scenario, this would perform HTTP requests to scrape web pages.
/// The delay simulates network latency and page processing time.
async fn web_scraper(url: &str) -> String {
    tokio::time::sleep(Duration::from_secs(2)).await;
    format!("Scraped content from: {url}")
}

/// Simulates a cheap "calculator" tool that returns instantly.
///
/// In a real scenario, this would perform mathematical computations.
/// No delay since computation is effectively instantaneous.
async fn calculator(expression: &str) -> String {
    // Simple expression evaluation simulation
    let result = match expression {
        "2 + 2" => "4",
        "10 * 5" => "50",
        "100 / 4" => "25",
        "7 * 8" => "56",
        "15 + 27" => "42",
        "99 - 33" => "66",
        "12 * 12" => "144",
        "256 / 16" => "16",
        _ => "unknown",
    };
    format!("{expression} = {result}")
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

    print_banner("Tool Concurrency");

    let api_key = require_env("GOOGLE_API_KEY")?;
    print_success(&format!("GOOGLE_API_KEY loaded ({} chars)", api_key.len()));
    println!();

    // -----------------------------------------------------------------------
    // Step 1: Configure Tool Concurrency Limits
    // -----------------------------------------------------------------------

    print_step(1, "Configure Tool Concurrency Limits");

    let config = RunConfig {
        tool_concurrency: ToolConcurrencyConfig {
            per_tool: HashMap::from([
                ("web_scraper".to_string(), 2),
                ("calculator".to_string(), 8),
            ]),
            backpressure: BackpressurePolicy::Queue,
        },
        model: "gemini-2.5-flash".to_string(),
    };

    print_success("RunConfig created with tool_concurrency_overrides:");
    println!("      web_scraper: max_concurrency = 2 (expensive, rate-limited)");
    println!("      calculator:  max_concurrency = 8 (cheap, instant)");
    println!("      backpressure_policy: Queue (wait for slot)");
    println!();

    // -----------------------------------------------------------------------
    // Step 2: Demonstrate BackpressurePolicy::Queue
    // -----------------------------------------------------------------------

    print_step(2, "Demonstrate BackpressurePolicy::Queue");
    print_progress("Dispatching 6 web_scraper calls with concurrency limit = 2");
    print_progress("Expected: 3 batches × 2s each ≈ 6s total (vs 2s if unlimited)\n");

    let queue_limiter = ConcurrencyLimiter::new(&config.tool_concurrency);

    let urls = [
        "https://example.com/page1",
        "https://example.com/page2",
        "https://example.com/page3",
        "https://example.com/page4",
        "https://example.com/page5",
        "https://example.com/page6",
    ];

    let start = Instant::now();

    // Dispatch all 6 calls concurrently — the limiter will queue them
    let futures: Vec<_> = urls
        .iter()
        .enumerate()
        .map(|(i, url)| {
            let limiter = &queue_limiter;
            let url = *url;
            async move {
                let call_start = Instant::now();
                let result = limiter
                    .execute("web_scraper", i + 1, web_scraper(url))
                    .await;
                let elapsed = call_start.elapsed();
                (i + 1, result, elapsed)
            }
        })
        .collect();

    let results = join_all(futures).await;
    let total_elapsed = start.elapsed();

    for (call_id, result, elapsed) in &results {
        match result {
            Ok(content) => {
                print_success(&format!(
                    "Call #{call_id} completed in {:.2}s: {content}",
                    elapsed.as_secs_f64()
                ));
            }
            Err(e) => {
                print_warning(&format!("Call #{call_id} failed: {e}"));
            }
        }
    }

    println!();
    print_success(&format!(
        "Total time for 6 queued web_scraper calls: {:.2}s",
        total_elapsed.as_secs_f64()
    ));
    print_progress("With unlimited concurrency, all 6 would complete in ~2s");
    print_progress(&format!(
        "With limit=2, they execute in ~3 batches of 2: ~{:.0}s",
        total_elapsed.as_secs_f64()
    ));
    println!();

    // Also demonstrate calculator with high concurrency
    print_progress("Dispatching 8 calculator calls with concurrency limit = 8");

    let expressions = [
        "2 + 2", "10 * 5", "100 / 4", "7 * 8", "15 + 27", "99 - 33", "12 * 12", "256 / 16",
    ];

    let calc_start = Instant::now();
    let calc_futures: Vec<_> = expressions
        .iter()
        .enumerate()
        .map(|(i, expr)| {
            let limiter = &queue_limiter;
            let expr = *expr;
            async move {
                limiter
                    .execute("calculator", i + 1, calculator(expr))
                    .await
            }
        })
        .collect();

    let calc_results = join_all(calc_futures).await;
    let calc_elapsed = calc_start.elapsed();

    for (i, result) in calc_results.iter().enumerate() {
        match result {
            Ok(content) => print_success(&format!("Calc #{}: {content}", i + 1)),
            Err(e) => print_warning(&format!("Calc #{}: {e}", i + 1)),
        }
    }

    println!();
    print_success(&format!(
        "Total time for 8 calculator calls: {:.4}s (all run concurrently, limit=8)",
        calc_elapsed.as_secs_f64()
    ));
    println!();

    // -----------------------------------------------------------------------
    // Step 3: Demonstrate BackpressurePolicy::Fail
    // -----------------------------------------------------------------------

    print_step(3, "Demonstrate BackpressurePolicy::Fail");
    print_progress("Dispatching 6 web_scraper calls with concurrency limit = 2");
    print_progress("Expected: 2 calls succeed immediately, 4 are rejected\n");

    let fail_config = ToolConcurrencyConfig {
        per_tool: HashMap::from([
            ("web_scraper".to_string(), 2),
            ("calculator".to_string(), 8),
        ]),
        backpressure: BackpressurePolicy::Fail,
    };

    let fail_limiter = Arc::new(ConcurrencyLimiter::new(&fail_config));

    let fail_start = Instant::now();

    // Under Fail policy, we need to spawn tasks so they actually race for permits
    let mut handles = Vec::new();
    for (i, url) in urls.iter().enumerate() {
        let limiter = Arc::clone(&fail_limiter);
        let url = url.to_string();
        let handle = tokio::spawn(async move {
            let call_start = Instant::now();
            let result = limiter
                .execute("web_scraper", i + 1, web_scraper(&url))
                .await;
            let elapsed = call_start.elapsed();
            (i + 1, result, elapsed)
        });
        handles.push(handle);
    }

    let fail_results: Vec<_> = join_all(handles)
        .await
        .into_iter()
        .map(|r| r.expect("task panicked"))
        .collect();

    let fail_elapsed = fail_start.elapsed();

    let mut succeeded = 0;
    let mut rejected = 0;

    for (call_id, result, elapsed) in &fail_results {
        match result {
            Ok(content) => {
                succeeded += 1;
                print_success(&format!(
                    "Call #{call_id} succeeded in {:.2}s: {content}",
                    elapsed.as_secs_f64()
                ));
            }
            Err(e) => {
                rejected += 1;
                print_warning(&format!(
                    "Call #{call_id} rejected in {:.4}s: {e}",
                    elapsed.as_secs_f64()
                ));
            }
        }
    }

    println!();
    print_success(&format!(
        "Fail policy results: {succeeded} succeeded, {rejected} rejected"
    ));
    print_success(&format!(
        "Total time: {:.2}s (rejected calls return instantly)",
        fail_elapsed.as_secs_f64()
    ));
    println!();

    // -----------------------------------------------------------------------
    // Summary
    // -----------------------------------------------------------------------

    print_summary(&[
        &format!(
            "Queue policy: 6 web_scraper calls with limit=2 took {:.2}s (~3 batches × 2s)",
            total_elapsed.as_secs_f64()
        ),
        &format!(
            "Queue policy: 8 calculator calls with limit=8 took {:.4}s (all concurrent)",
            calc_elapsed.as_secs_f64()
        ),
        &format!(
            "Fail policy: {succeeded} calls succeeded, {rejected} rejected immediately"
        ),
        "Per-tool limits let you throttle expensive tools without blocking cheap ones.",
        "Queue policy ensures all calls eventually complete (higher latency).",
        "Fail policy provides fast feedback when system is overloaded.",
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
        // Use a variable name that is guaranteed not to exist
        let result = require_env("ADK_TEST_NONEXISTENT_VAR_12345");
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("ADK_TEST_NONEXISTENT_VAR_12345"),
            "Error should contain the variable name, got: {msg}"
        );
        assert!(
            msg.contains(".env.example"),
            "Error should reference .env.example, got: {msg}"
        );
    }

    #[test]
    fn test_require_env_present_variable() {
        // SAFETY: This test runs in isolation and does not race with other threads
        // reading this specific environment variable.
        unsafe { std::env::set_var("ADK_TEST_PRESENT_VAR", "test_value") };
        let result = require_env("ADK_TEST_PRESENT_VAR");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test_value");
        unsafe { std::env::remove_var("ADK_TEST_PRESENT_VAR") };
    }

    #[test]
    fn test_classify_llm_error_auth() {
        let err = anyhow::anyhow!("HTTP 401 Unauthorized");
        let classification = classify_llm_error(&err);
        assert!(
            classification.contains("Authentication"),
            "Should classify as auth error, got: {classification}"
        );
    }

    #[test]
    fn test_classify_llm_error_rate_limit() {
        let err = anyhow::anyhow!("HTTP 429 rate limit exceeded");
        let classification = classify_llm_error(&err);
        assert!(
            classification.contains("Rate limited"),
            "Should classify as rate limit, got: {classification}"
        );
    }

    #[test]
    fn test_classify_llm_error_context_length() {
        let err = anyhow::anyhow!("context length exceeded maximum token limit");
        let classification = classify_llm_error(&err);
        assert!(
            classification.contains("Context too large"),
            "Should classify as context error, got: {classification}"
        );
    }

    #[test]
    fn test_classify_llm_error_unknown() {
        let err = anyhow::anyhow!("some random network error");
        let classification = classify_llm_error(&err);
        assert!(
            classification.contains("Unexpected error"),
            "Should classify as unexpected, got: {classification}"
        );
    }

    #[test]
    fn test_backpressure_policy_display() {
        assert_eq!(format!("{}", BackpressurePolicy::Queue), "Queue");
        assert_eq!(format!("{}", BackpressurePolicy::Fail), "Fail");
    }

    #[tokio::test]
    async fn test_queue_policy_all_calls_succeed() {
        let config = ToolConcurrencyConfig {
            per_tool: HashMap::from([("test_tool".to_string(), 2)]),
            backpressure: BackpressurePolicy::Queue,
        };
        let limiter = ConcurrencyLimiter::new(&config);

        // Dispatch 4 calls with limit=2 — all should succeed (queued)
        let futures: Vec<_> = (0..4)
            .map(|i| {
                limiter.execute("test_tool", i, async move {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    format!("result_{i}")
                })
            })
            .collect();

        let results = join_all(futures).await;
        for result in &results {
            assert!(result.is_ok(), "All queued calls should succeed");
        }
        assert_eq!(results.len(), 4);
    }

    #[tokio::test]
    async fn test_fail_policy_rejects_excess_calls() {
        let config = ToolConcurrencyConfig {
            per_tool: HashMap::from([("test_tool".to_string(), 1)]),
            backpressure: BackpressurePolicy::Fail,
        };
        let limiter = Arc::new(ConcurrencyLimiter::new(&config));

        // Spawn tasks that hold the semaphore for a while
        let limiter_clone = Arc::clone(&limiter);
        let hold_task = tokio::spawn(async move {
            limiter_clone
                .execute("test_tool", 1, async {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    "held"
                })
                .await
        });

        // Give the first task time to acquire the permit
        tokio::time::sleep(Duration::from_millis(10)).await;

        // This call should be rejected immediately since limit=1 and it's occupied
        let result = limiter
            .execute("test_tool", 2, async { "should_not_run" })
            .await;

        assert!(result.is_err(), "Excess call should be rejected under Fail policy");
        assert!(
            result.unwrap_err().contains("Concurrency limit exceeded"),
            "Error should mention concurrency limit"
        );

        // Clean up
        let _ = hold_task.await;
    }

    #[tokio::test]
    async fn test_web_scraper_has_delay() {
        let start = Instant::now();
        let result = web_scraper("https://test.com").await;
        let elapsed = start.elapsed();

        assert!(elapsed >= Duration::from_secs(2), "web_scraper should take at least 2s");
        assert!(result.contains("https://test.com"));
    }

    #[tokio::test]
    async fn test_calculator_is_instant() {
        let start = Instant::now();
        let result = calculator("2 + 2").await;
        let elapsed = start.elapsed();

        assert!(elapsed < Duration::from_millis(10), "calculator should be near-instant");
        assert_eq!(result, "2 + 2 = 4");
    }

    #[tokio::test]
    async fn test_concurrency_limit_enforces_batching() {
        // With limit=2 and 4 calls each taking 100ms, total should be ~200ms (2 batches)
        let config = ToolConcurrencyConfig {
            per_tool: HashMap::from([("slow_tool".to_string(), 2)]),
            backpressure: BackpressurePolicy::Queue,
        };
        let limiter = ConcurrencyLimiter::new(&config);

        let start = Instant::now();
        let futures: Vec<_> = (0..4)
            .map(|i| {
                limiter.execute("slow_tool", i, async move {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    i
                })
            })
            .collect();

        let results = join_all(futures).await;
        let elapsed = start.elapsed();

        // All should succeed
        assert!(results.iter().all(|r| r.is_ok()));
        // Should take at least 200ms (2 batches of 100ms) but less than 400ms
        assert!(
            elapsed >= Duration::from_millis(180),
            "Should take at least ~200ms for 2 batches, got {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(400),
            "Should not take more than ~400ms, got {elapsed:?}"
        );
    }
}
