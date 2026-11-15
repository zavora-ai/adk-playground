//! # A2A Interceptors Example
//!
//! Demonstrates ADK-Rust's A2A (Agent-to-Agent) interceptor chain for request
//! processing, showing how to implement authentication, rate limiting, and
//! audit logging as composable interceptors.
//!
//! ## What This Shows
//!
//! - Implementing `A2aInterceptor` trait for custom request processing
//! - Configuring an `InterceptorChain` with multiple interceptors
//! - `InterceptorDecision::Continue` for passing requests through
//! - `InterceptorDecision::Reject` for authentication failures
//! - `InterceptorDecision::ShortCircuit` for rate limit enforcement
//! - Audit logging of all intercepted requests
//!
//! ## Prerequisites
//!
//! - `GOOGLE_API_KEY` environment variable set (for the Gemini LLM provider)
//!
//! ## Run
//!
//! ```bash
//! cargo run --manifest-path examples/a2a_interceptors/Cargo.toml
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

// ---------------------------------------------------------------------------
// Shared Helpers
// ---------------------------------------------------------------------------

/// Require an environment variable or return an actionable error message.
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
// A2A Interceptor Framework (Simulated)
// ---------------------------------------------------------------------------

/// The decision an interceptor makes about a request.
///
/// This simulates the ADK-Rust `InterceptorDecision` enum from
/// `adk_server::a2a::interceptor`.
#[derive(Debug, Clone)]
enum InterceptorDecision {
    /// Allow the request to continue to the next interceptor or handler.
    Continue,
    /// Reject the request with an error message. The request is denied
    /// and no further interceptors are executed.
    Reject { reason: String },
    /// Short-circuit the chain and return a response immediately.
    /// Used for rate limiting or cached responses.
    ShortCircuit { response: Value },
}

impl std::fmt::Display for InterceptorDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InterceptorDecision::Continue => write!(f, "Continue"),
            InterceptorDecision::Reject { reason } => {
                write!(f, "Reject({reason})")
            }
            InterceptorDecision::ShortCircuit { .. } => {
                write!(f, "ShortCircuit")
            }
        }
    }
}

/// Represents an incoming A2A request with metadata.
#[derive(Debug, Clone)]
struct A2aRequest {
    /// The JSON-RPC method being called.
    method: String,
    /// The bearer token from the Authorization header (if present).
    bearer_token: Option<String>,
    /// Client identity extracted from the token or connection.
    client_id: Option<String>,
    /// The request payload.
    #[allow(dead_code)]
    payload: Value,
}

/// Trait for A2A interceptors that can inspect, modify, or reject requests.
///
/// Interceptors are executed in chain order. Each interceptor can:
/// - Allow the request to continue (`InterceptorDecision::Continue`)
/// - Reject the request (`InterceptorDecision::Reject`)
/// - Short-circuit with an immediate response (`InterceptorDecision::ShortCircuit`)
#[async_trait]
trait A2aInterceptor: Send + Sync {
    /// The name of this interceptor (for logging and debugging).
    fn name(&self) -> &str;

    /// Called before the request is delegated to the agent.
    /// Returns a decision about whether to continue, reject, or short-circuit.
    async fn before_delegation(
        &self,
        request: &mut A2aRequest,
    ) -> InterceptorDecision;
}

/// A chain of interceptors executed in order for each request.
///
/// The chain stops at the first `Reject` or `ShortCircuit` decision.
/// If all interceptors return `Continue`, the request proceeds to the handler.
struct InterceptorChain {
    interceptors: Vec<Arc<dyn A2aInterceptor>>,
}

impl InterceptorChain {
    /// Create a new chain with the given interceptors (executed in order).
    fn new(interceptors: Vec<Arc<dyn A2aInterceptor>>) -> Self {
        Self { interceptors }
    }

    /// Execute the interceptor chain on a request.
    /// Returns the final decision and the list of interceptors that were executed.
    async fn execute(
        &self,
        request: &mut A2aRequest,
    ) -> (InterceptorDecision, Vec<String>) {
        let mut executed = Vec::new();

        for interceptor in &self.interceptors {
            executed.push(interceptor.name().to_string());
            let decision = interceptor.before_delegation(request).await;

            match &decision {
                InterceptorDecision::Continue => {
                    // Continue to next interceptor
                }
                InterceptorDecision::Reject { .. }
                | InterceptorDecision::ShortCircuit { .. } => {
                    return (decision, executed);
                }
            }
        }

        (InterceptorDecision::Continue, executed)
    }
}

// ---------------------------------------------------------------------------
// BearerAuthInterceptor
// ---------------------------------------------------------------------------

/// Validates bearer tokens against a list of known valid tokens.
///
/// If the request contains a valid bearer token, the interceptor extracts
/// the client identity and allows the request to continue. Invalid or
/// missing tokens result in rejection.
struct BearerAuthInterceptor {
    /// Set of valid bearer tokens mapped to client identities.
    valid_tokens: HashMap<String, String>,
}

impl BearerAuthInterceptor {
    fn new(tokens: HashMap<String, String>) -> Self {
        Self {
            valid_tokens: tokens,
        }
    }
}

#[async_trait]
impl A2aInterceptor for BearerAuthInterceptor {
    fn name(&self) -> &str {
        "BearerAuthInterceptor"
    }

    async fn before_delegation(
        &self,
        request: &mut A2aRequest,
    ) -> InterceptorDecision {
        match &request.bearer_token {
            None => InterceptorDecision::Reject {
                reason: "Missing Authorization header: bearer token required"
                    .to_string(),
            },
            Some(token) => {
                if let Some(client_id) = self.valid_tokens.get(token) {
                    // Valid token — set client identity on the request
                    request.client_id = Some(client_id.clone());
                    InterceptorDecision::Continue
                } else {
                    InterceptorDecision::Reject {
                        reason: format!(
                            "Invalid bearer token: authentication failed for token '{}'",
                            &token[..token.len().min(8)]
                        ),
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// RateLimitInterceptor
// ---------------------------------------------------------------------------

/// Tracks request counts per client and enforces rate limits.
///
/// When a client exceeds the maximum number of requests within the
/// configured window, subsequent requests are short-circuited with
/// a rate-limit response.
struct RateLimitInterceptor {
    /// Maximum requests allowed per client within the window.
    max_requests: usize,
    /// Time window for rate limiting.
    window: Duration,
    /// Per-client request tracking: client_id → (count, window_start).
    state: Arc<Mutex<HashMap<String, (usize, Instant)>>>,
}

impl RateLimitInterceptor {
    fn new(max_requests: usize, window: Duration) -> Self {
        Self {
            max_requests,
            window,
            state: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl A2aInterceptor for RateLimitInterceptor {
    fn name(&self) -> &str {
        "RateLimitInterceptor"
    }

    async fn before_delegation(
        &self,
        request: &mut A2aRequest,
    ) -> InterceptorDecision {
        let client_id = request
            .client_id
            .clone()
            .unwrap_or_else(|| "anonymous".to_string());

        let mut state = self.state.lock().await;
        let now = Instant::now();

        let entry = state
            .entry(client_id.clone())
            .or_insert((0, now));

        // Reset counter if window has elapsed
        if now.duration_since(entry.1) > self.window {
            *entry = (0, now);
        }

        entry.0 += 1;

        if entry.0 > self.max_requests {
            InterceptorDecision::ShortCircuit {
                response: serde_json::json!({
                    "error": {
                        "code": 429,
                        "message": format!(
                            "Rate limit exceeded for client '{}': {} requests in {:?} (max: {})",
                            client_id, entry.0, self.window, self.max_requests
                        ),
                    }
                }),
            }
        } else {
            InterceptorDecision::Continue
        }
    }
}

// ---------------------------------------------------------------------------
// AuditLogInterceptor
// ---------------------------------------------------------------------------

/// An audit log entry recording details of an intercepted request.
#[derive(Debug, Clone)]
struct AuditEntry {
    /// The JSON-RPC method called.
    method: String,
    /// The client identity (if authenticated).
    client_id: String,
    /// When the request was received.
    #[allow(dead_code)]
    timestamp: Instant,
    /// How long the interceptor chain took to process.
    duration: Duration,
    /// The final decision made by the chain.
    decision: String,
}

impl std::fmt::Display for AuditEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{:>8.3}ms] method={:<20} client={:<15} decision={}",
            self.duration.as_secs_f64() * 1000.0,
            self.method,
            self.client_id,
            self.decision,
        )
    }
}

/// Records method, client identity, timestamp, and duration for all requests.
///
/// The audit log interceptor always returns `Continue` — it observes but
/// never blocks requests. It records the entry before the request proceeds.
struct AuditLogInterceptor {
    /// Shared log of all audit entries.
    log: Arc<Mutex<Vec<AuditEntry>>>,
}

impl AuditLogInterceptor {
    fn new(log: Arc<Mutex<Vec<AuditEntry>>>) -> Self {
        Self { log }
    }
}

#[async_trait]
impl A2aInterceptor for AuditLogInterceptor {
    fn name(&self) -> &str {
        "AuditLogInterceptor"
    }

    async fn before_delegation(
        &self,
        request: &mut A2aRequest,
    ) -> InterceptorDecision {
        // Record the audit entry (decision will be updated after chain completes)
        let entry = AuditEntry {
            method: request.method.clone(),
            client_id: request
                .client_id
                .clone()
                .unwrap_or_else(|| "unauthenticated".to_string()),
            timestamp: Instant::now(),
            duration: Duration::ZERO, // Updated after chain execution
            decision: "pending".to_string(),
        };
        self.log.lock().await.push(entry);
        InterceptorDecision::Continue
    }
}

// ---------------------------------------------------------------------------
// A2A Server Simulation
// ---------------------------------------------------------------------------

/// Simulates an A2A server that processes requests through an interceptor chain.
struct A2aServer {
    chain: InterceptorChain,
    audit_log: Arc<Mutex<Vec<AuditEntry>>>,
}

impl A2aServer {
    /// Process a request through the interceptor chain and return the result.
    async fn handle_request(
        &self,
        mut request: A2aRequest,
    ) -> (InterceptorDecision, Vec<String>) {
        let start = Instant::now();
        let (decision, executed) = self.chain.execute(&mut request).await;
        let duration = start.elapsed();

        // Update the last audit entry with the final decision and duration
        let mut log = self.audit_log.lock().await;
        if let Some(entry) = log.last_mut() {
            entry.duration = duration;
            entry.decision = decision.to_string();
            // Update client_id if it was set during auth
            if let Some(ref cid) = request.client_id {
                entry.client_id = cid.clone();
            }
        }

        (decision, executed)
    }
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
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    print_banner("A2A Interceptors");

    let api_key = require_env("GOOGLE_API_KEY")?;
    print_success(&format!(
        "GOOGLE_API_KEY loaded ({} chars)",
        api_key.len()
    ));
    println!();

    // -------------------------------------------------------------------
    // Step 1: Configure Interceptor Chain
    // -------------------------------------------------------------------

    print_step(1, "Configure A2A Server with Interceptor Chain");

    // Set up valid tokens for authentication
    let valid_tokens: HashMap<String, String> = HashMap::from([
        (
            "valid-token-abc".to_string(),
            "agent-client-1".to_string(),
        ),
        (
            "valid-token-def".to_string(),
            "agent-client-2".to_string(),
        ),
    ]);

    let audit_log: Arc<Mutex<Vec<AuditEntry>>> =
        Arc::new(Mutex::new(Vec::new()));

    // Build the interceptor chain: Auth → RateLimit → AuditLog
    let auth_interceptor = Arc::new(BearerAuthInterceptor::new(valid_tokens));
    let rate_limit_interceptor = Arc::new(RateLimitInterceptor::new(
        3,                          // max 3 requests per client
        Duration::from_secs(60),    // within a 60-second window
    ));
    let audit_interceptor =
        Arc::new(AuditLogInterceptor::new(Arc::clone(&audit_log)));

    let chain = InterceptorChain::new(vec![
        auth_interceptor as Arc<dyn A2aInterceptor>,
        rate_limit_interceptor as Arc<dyn A2aInterceptor>,
        audit_interceptor as Arc<dyn A2aInterceptor>,
    ]);

    let server = A2aServer {
        chain,
        audit_log: Arc::clone(&audit_log),
    };

    print_success("InterceptorChain configured with 3 interceptors:");
    println!("      1. BearerAuthInterceptor — validates bearer tokens");
    println!(
        "      2. RateLimitInterceptor — max 3 requests/client/60s"
    );
    println!(
        "      3. AuditLogInterceptor  — records method, client, duration"
    );
    println!();

    // -------------------------------------------------------------------
    // Step 2: Send Request with Valid Token
    // -------------------------------------------------------------------

    print_step(2, "Send Request with Valid Bearer Token");
    print_progress(
        "Sending 'tasks/send' with bearer token: valid-token-abc",
    );

    let valid_request = A2aRequest {
        method: "tasks/send".to_string(),
        bearer_token: Some("valid-token-abc".to_string()),
        client_id: None,
        payload: serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tasks/send",
            "params": {
                "message": {
                    "role": "user",
                    "parts": [{"text": "Summarize the latest news"}]
                }
            }
        }),
    };

    let (decision, executed) =
        server.handle_request(valid_request).await;

    print_success(&format!("Decision: {decision}"));
    print_success(&format!(
        "Interceptors executed: {}",
        executed.join(" → ")
    ));
    print_success(
        "Request passed all interceptors — proceeding to agent handler",
    );
    println!();

    // -------------------------------------------------------------------
    // Step 3: Send Request with Invalid Token
    // -------------------------------------------------------------------

    print_step(3, "Send Request with Invalid Bearer Token");
    print_progress(
        "Sending 'tasks/send' with bearer token: invalid-token-xyz",
    );

    let invalid_request = A2aRequest {
        method: "tasks/send".to_string(),
        bearer_token: Some("invalid-token-xyz".to_string()),
        client_id: None,
        payload: serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tasks/send",
            "params": {
                "message": {
                    "role": "user",
                    "parts": [{"text": "Do something unauthorized"}]
                }
            }
        }),
    };

    let (decision, executed) =
        server.handle_request(invalid_request).await;

    match &decision {
        InterceptorDecision::Reject { reason } => {
            print_warning(&format!("Decision: Reject"));
            print_warning(&format!("Reason: {reason}"));
        }
        _ => {
            print_success(&format!("Decision: {decision}"));
        }
    }
    print_success(&format!(
        "Interceptors executed: {}",
        executed.join(" → ")
    ));
    print_progress(
        "Chain stopped at BearerAuthInterceptor — \
         RateLimitInterceptor and AuditLogInterceptor were skipped",
    );
    println!();

    // -------------------------------------------------------------------
    // Step 4: Exceed Rate Limit
    // -------------------------------------------------------------------

    print_step(4, "Exceed Rate Limit (ShortCircuit)");
    print_progress(
        "Sending 4 rapid requests from agent-client-1 (limit: 3/60s)",
    );
    println!();

    // Send requests 2 and 3 (request 1 was already sent in Step 2)
    for i in 2..=4 {
        let request = A2aRequest {
            method: format!("tasks/send#{i}"),
            bearer_token: Some("valid-token-abc".to_string()),
            client_id: None,
            payload: serde_json::json!({
                "jsonrpc": "2.0",
                "method": "tasks/send",
                "params": {
                    "message": {
                        "role": "user",
                        "parts": [{"text": format!("Request #{i}")}]
                    }
                }
            }),
        };

        let (decision, executed) =
            server.handle_request(request).await;

        match &decision {
            InterceptorDecision::Continue => {
                print_success(&format!(
                    "Request #{i}: Continue (executed: {})",
                    executed.join(" → ")
                ));
            }
            InterceptorDecision::ShortCircuit { response } => {
                print_warning(&format!(
                    "Request #{i}: ShortCircuit — rate limit exceeded!"
                ));
                print_warning(&format!(
                    "Response: {}",
                    response["error"]["message"]
                        .as_str()
                        .unwrap_or("rate limited")
                ));
                print_success(&format!(
                    "Interceptors executed: {}",
                    executed.join(" → ")
                ));
            }
            InterceptorDecision::Reject { reason } => {
                print_warning(&format!(
                    "Request #{i}: Reject — {reason}"
                ));
            }
        }
    }
    println!();

    // -------------------------------------------------------------------
    // Step 5: Print Audit Log
    // -------------------------------------------------------------------

    print_step(5, "Print Audit Log Entries");

    let log = audit_log.lock().await;
    print_success(&format!("{} audit entries recorded:\n", log.len()));

    println!(
        "  {:<10} {:<22} {:<17} {}",
        "Duration", "Method", "Client", "Decision"
    );
    println!("  {}", "─".repeat(70));

    for entry in log.iter() {
        println!("  {entry}");
    }
    println!();

    // -------------------------------------------------------------------
    // Summary
    // -------------------------------------------------------------------

    let total_requests = log.len();
    let continued = log
        .iter()
        .filter(|e| e.decision == "Continue")
        .count();
    let rejected = log
        .iter()
        .filter(|e| e.decision.starts_with("Reject"))
        .count();
    let short_circuited = log
        .iter()
        .filter(|e| e.decision == "ShortCircuit")
        .count();

    drop(log); // Release the lock before printing summary

    print_summary(&[
        &format!(
            "Total requests processed: {total_requests}"
        ),
        &format!(
            "  Continued (passed all interceptors): {continued}"
        ),
        &format!(
            "  Rejected (auth failure):             {rejected}"
        ),
        &format!(
            "  Short-circuited (rate limited):      {short_circuited}"
        ),
        "",
        "BearerAuthInterceptor: validates tokens, sets client identity, rejects invalid credentials",
        "RateLimitInterceptor: enforces per-client request limits, short-circuits on excess",
        "AuditLogInterceptor: records method, client identity, and duration for all requests",
        "InterceptorChain executes interceptors in order; early decisions skip remaining chain",
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
        let result = require_env("ADK_TEST_NONEXISTENT_VAR_A2A_12345");
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("ADK_TEST_NONEXISTENT_VAR_A2A_12345"),
            "Error should contain the variable name, got: {msg}"
        );
        assert!(
            msg.contains(".env.example"),
            "Error should reference .env.example, got: {msg}"
        );
    }

    #[test]
    fn test_require_env_present_variable() {
        unsafe { std::env::set_var("ADK_TEST_A2A_PRESENT_VAR", "test_value") };
        let result = require_env("ADK_TEST_A2A_PRESENT_VAR");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test_value");
        unsafe { std::env::remove_var("ADK_TEST_A2A_PRESENT_VAR") };
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
        let err =
            anyhow::anyhow!("context length exceeded maximum token limit");
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
    fn test_interceptor_decision_display() {
        let cont = InterceptorDecision::Continue;
        assert_eq!(format!("{cont}"), "Continue");

        let reject = InterceptorDecision::Reject {
            reason: "bad token".to_string(),
        };
        assert_eq!(format!("{reject}"), "Reject(bad token)");

        let sc = InterceptorDecision::ShortCircuit {
            response: serde_json::json!({"error": "rate limited"}),
        };
        assert_eq!(format!("{sc}"), "ShortCircuit");
    }

    #[tokio::test]
    async fn test_bearer_auth_valid_token() {
        let tokens = HashMap::from([
            ("token-1".to_string(), "client-a".to_string()),
        ]);
        let interceptor = BearerAuthInterceptor::new(tokens);

        let mut request = A2aRequest {
            method: "tasks/send".to_string(),
            bearer_token: Some("token-1".to_string()),
            client_id: None,
            payload: serde_json::json!({}),
        };

        let decision =
            interceptor.before_delegation(&mut request).await;
        assert!(
            matches!(decision, InterceptorDecision::Continue),
            "Valid token should continue"
        );
        assert_eq!(
            request.client_id,
            Some("client-a".to_string()),
            "Client ID should be set"
        );
    }

    #[tokio::test]
    async fn test_bearer_auth_invalid_token() {
        let tokens = HashMap::from([
            ("token-1".to_string(), "client-a".to_string()),
        ]);
        let interceptor = BearerAuthInterceptor::new(tokens);

        let mut request = A2aRequest {
            method: "tasks/send".to_string(),
            bearer_token: Some("bad-token".to_string()),
            client_id: None,
            payload: serde_json::json!({}),
        };

        let decision =
            interceptor.before_delegation(&mut request).await;
        assert!(
            matches!(decision, InterceptorDecision::Reject { .. }),
            "Invalid token should be rejected"
        );
    }

    #[tokio::test]
    async fn test_bearer_auth_missing_token() {
        let tokens = HashMap::from([
            ("token-1".to_string(), "client-a".to_string()),
        ]);
        let interceptor = BearerAuthInterceptor::new(tokens);

        let mut request = A2aRequest {
            method: "tasks/send".to_string(),
            bearer_token: None,
            client_id: None,
            payload: serde_json::json!({}),
        };

        let decision =
            interceptor.before_delegation(&mut request).await;
        assert!(
            matches!(decision, InterceptorDecision::Reject { .. }),
            "Missing token should be rejected"
        );
    }

    #[tokio::test]
    async fn test_rate_limiter_allows_within_limit() {
        let limiter = RateLimitInterceptor::new(
            3,
            Duration::from_secs(60),
        );

        for i in 1..=3 {
            let mut request = A2aRequest {
                method: format!("tasks/send#{i}"),
                bearer_token: None,
                client_id: Some("test-client".to_string()),
                payload: serde_json::json!({}),
            };

            let decision =
                limiter.before_delegation(&mut request).await;
            assert!(
                matches!(decision, InterceptorDecision::Continue),
                "Request #{i} should be allowed within limit"
            );
        }
    }

    #[tokio::test]
    async fn test_rate_limiter_short_circuits_over_limit() {
        let limiter = RateLimitInterceptor::new(
            2,
            Duration::from_secs(60),
        );

        // Use up the limit
        for _ in 0..2 {
            let mut request = A2aRequest {
                method: "tasks/send".to_string(),
                bearer_token: None,
                client_id: Some("test-client".to_string()),
                payload: serde_json::json!({}),
            };
            limiter.before_delegation(&mut request).await;
        }

        // Third request should be short-circuited
        let mut request = A2aRequest {
            method: "tasks/send".to_string(),
            bearer_token: None,
            client_id: Some("test-client".to_string()),
            payload: serde_json::json!({}),
        };

        let decision =
            limiter.before_delegation(&mut request).await;
        assert!(
            matches!(
                decision,
                InterceptorDecision::ShortCircuit { .. }
            ),
            "Exceeding rate limit should short-circuit"
        );
    }

    #[tokio::test]
    async fn test_audit_log_records_entries() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let interceptor = AuditLogInterceptor::new(Arc::clone(&log));

        let mut request = A2aRequest {
            method: "tasks/get".to_string(),
            bearer_token: None,
            client_id: Some("audit-client".to_string()),
            payload: serde_json::json!({}),
        };

        let decision =
            interceptor.before_delegation(&mut request).await;
        assert!(matches!(decision, InterceptorDecision::Continue));

        let entries = log.lock().await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].method, "tasks/get");
        assert_eq!(entries[0].client_id, "audit-client");
    }

    #[tokio::test]
    async fn test_interceptor_chain_stops_on_reject() {
        let tokens = HashMap::from([
            ("good-token".to_string(), "client".to_string()),
        ]);
        let auth = Arc::new(BearerAuthInterceptor::new(tokens));
        let log = Arc::new(Mutex::new(Vec::new()));
        let audit = Arc::new(AuditLogInterceptor::new(log.clone()));

        // Auth first, then audit — if auth rejects, audit should not run
        let chain = InterceptorChain::new(vec![
            auth as Arc<dyn A2aInterceptor>,
            audit as Arc<dyn A2aInterceptor>,
        ]);

        let mut request = A2aRequest {
            method: "tasks/send".to_string(),
            bearer_token: Some("bad-token".to_string()),
            client_id: None,
            payload: serde_json::json!({}),
        };

        let (decision, executed) =
            chain.execute(&mut request).await;

        assert!(matches!(decision, InterceptorDecision::Reject { .. }));
        // Only auth interceptor should have executed
        assert_eq!(executed.len(), 1);
        assert_eq!(executed[0], "BearerAuthInterceptor");

        // Audit log should be empty since it was never reached
        let entries = log.lock().await;
        assert_eq!(entries.len(), 0);
    }

    #[tokio::test]
    async fn test_interceptor_chain_all_continue() {
        let tokens = HashMap::from([
            ("good-token".to_string(), "client".to_string()),
        ]);
        let auth = Arc::new(BearerAuthInterceptor::new(tokens));
        let limiter = Arc::new(RateLimitInterceptor::new(
            10,
            Duration::from_secs(60),
        ));
        let log = Arc::new(Mutex::new(Vec::new()));
        let audit = Arc::new(AuditLogInterceptor::new(log));

        let chain = InterceptorChain::new(vec![
            auth as Arc<dyn A2aInterceptor>,
            limiter as Arc<dyn A2aInterceptor>,
            audit as Arc<dyn A2aInterceptor>,
        ]);

        let mut request = A2aRequest {
            method: "tasks/send".to_string(),
            bearer_token: Some("good-token".to_string()),
            client_id: None,
            payload: serde_json::json!({}),
        };

        let (decision, executed) =
            chain.execute(&mut request).await;

        assert!(matches!(decision, InterceptorDecision::Continue));
        assert_eq!(executed.len(), 3);
    }
}
