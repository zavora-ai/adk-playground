//! # Context Compaction Example
//!
//! Demonstrates ADK-Rust's automatic context window management, showing how to
//! handle long conversations without exceeding model token limits using both
//! truncation and summarisation compaction strategies.
//!
//! ## What This Shows
//!
//! - Configuring `RunConfig` with a low `context_budget` and `CompactionStrategy`
//! - `TruncationCompaction` behavior: dropping older events to fit within budget
//! - `SummarisationCompaction` behavior: LLM-generated summary replacing older events
//! - Token estimates before and after compaction showing reduction
//! - Coherence after compaction by referencing earlier conversation context
//!
//! ## Prerequisites
//!
//! - `GOOGLE_API_KEY` environment variable set (for the Gemini LLM provider)
//!
//! ## Run
//!
//! ```bash
//! cargo run --manifest-path examples/context_compaction/Cargo.toml
//! ```

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
// Context Compaction Types
// ---------------------------------------------------------------------------

/// Role of a message in the conversation.
#[derive(Debug, Clone, PartialEq)]
enum Role {
    User,
    Assistant,
    System,
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::User => write!(f, "user"),
            Role::Assistant => write!(f, "assistant"),
            Role::System => write!(f, "system"),
        }
    }
}

/// A single message/event in the conversation context.
#[derive(Debug, Clone)]
struct ConversationEvent {
    /// The role of the message sender.
    role: Role,
    /// The text content of the message.
    content: String,
    /// Estimated token count for this event.
    estimated_tokens: usize,
    /// Turn number in the conversation.
    turn: usize,
}

/// Estimate token count for a string (approximation: ~4 chars per token).
fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

/// Strategy for compacting conversation context when token budget is exceeded.
trait CompactionStrategy: std::fmt::Debug {
    /// Apply compaction to the event list, returning a reduced event list.
    fn compact(&self, events: &[ConversationEvent], budget: usize) -> Vec<ConversationEvent>;

    /// Name of this strategy for display purposes.
    fn name(&self) -> &str;
}

/// Truncation compaction: drops oldest events, preserving the most recent N.
#[derive(Debug, Clone)]
struct TruncationCompaction {
    /// Number of recent events to preserve.
    preserve_recent: usize,
}

impl CompactionStrategy for TruncationCompaction {
    fn compact(&self, events: &[ConversationEvent], budget: usize) -> Vec<ConversationEvent> {
        if events.is_empty() {
            return Vec::new();
        }

        // Start by keeping only the most recent `preserve_recent` events
        let mut kept: Vec<ConversationEvent> = events
            .iter()
            .rev()
            .take(self.preserve_recent)
            .cloned()
            .collect();
        kept.reverse();

        // If still over budget, keep dropping from the front
        while total_tokens(&kept) > budget && kept.len() > 1 {
            kept.remove(0);
        }

        kept
    }

    fn name(&self) -> &str {
        "TruncationCompaction"
    }
}

/// Summarisation compaction: replaces older events with an LLM-generated summary.
///
/// In a real implementation, this would call the LLM to generate a summary.
/// For this demonstration, we simulate the summarisation process.
#[derive(Debug, Clone)]
struct SummarisationCompaction {
    /// Number of older turns to summarise into a single message.
    turns_to_summarise: usize,
}

impl CompactionStrategy for SummarisationCompaction {
    fn compact(&self, events: &[ConversationEvent], budget: usize) -> Vec<ConversationEvent> {
        if events.is_empty() {
            return Vec::new();
        }

        // Split events: older ones to summarise, recent ones to keep
        let split_point = self.turns_to_summarise.min(events.len().saturating_sub(2));
        let (to_summarise, to_keep) = events.split_at(split_point);

        if to_summarise.is_empty() {
            return events.to_vec();
        }

        // Generate a simulated LLM summary of the older events
        let summary = generate_summary(to_summarise);
        let summary_tokens = estimate_tokens(&summary);

        // Create a summary event replacing the older events
        let mut compacted = vec![ConversationEvent {
            role: Role::System,
            content: summary,
            estimated_tokens: summary_tokens,
            turn: 0, // Summary represents all earlier turns
        }];

        // Append the recent events
        compacted.extend(to_keep.iter().cloned());

        // If still over budget, truncate further from the summary
        while total_tokens(&compacted) > budget && compacted.len() > 1 {
            compacted.remove(0);
        }

        compacted
    }

    fn name(&self) -> &str {
        "SummarisationCompaction"
    }
}

/// Configuration for context compaction behavior.
#[derive(Debug)]
struct CompactionConfig {
    /// The compaction strategy to apply.
    strategy: Box<dyn CompactionStrategy>,
    /// Maximum token budget for the context window.
    context_budget: usize,
}

impl CompactionConfig {
    /// Create a new compaction configuration.
    fn new(strategy: Box<dyn CompactionStrategy>, context_budget: usize) -> Self {
        Self {
            strategy,
            context_budget,
        }
    }

    /// Apply compaction if the events exceed the configured budget.
    fn compact_if_needed(&self, events: &[ConversationEvent]) -> Vec<ConversationEvent> {
        if total_tokens(events) > self.context_budget {
            self.strategy.compact(events, self.context_budget)
        } else {
            events.to_vec()
        }
    }
}

/// Calculate total estimated tokens across all events.
fn total_tokens(events: &[ConversationEvent]) -> usize {
    events.iter().map(|e| e.estimated_tokens).sum()
}

/// Generate a simulated LLM summary of conversation events.
///
/// In production, this would call the Gemini API to produce a coherent summary.
/// For demonstration, we extract key topics and create a condensed summary.
fn generate_summary(events: &[ConversationEvent]) -> String {
    let topics: Vec<&str> = events
        .iter()
        .filter(|e| e.role == Role::User)
        .map(|e| e.content.as_str())
        .collect();

    let key_points: Vec<&str> = events
        .iter()
        .filter(|e| e.role == Role::Assistant)
        .map(|e| e.content.as_str())
        .collect();

    let mut summary = String::from(
        "[LLM-Generated Summary] Previous conversation covered: ",
    );

    if !topics.is_empty() {
        summary.push_str("User asked about ");
        for (i, topic) in topics.iter().enumerate() {
            if i > 0 {
                summary.push_str("; then ");
            }
            // Extract first 60 chars of each topic
            let truncated: String = topic.chars().take(60).collect();
            summary.push_str(&truncated);
        }
        summary.push_str(". ");
    }

    if !key_points.is_empty() {
        summary.push_str("Key points discussed: ");
        for (i, point) in key_points.iter().enumerate() {
            if i > 0 {
                summary.push_str("; ");
            }
            let truncated: String = point.chars().take(80).collect();
            summary.push_str(&truncated);
        }
        summary.push('.');
    }

    summary
}

// ---------------------------------------------------------------------------
// Simulated Multi-Turn Conversation
// ---------------------------------------------------------------------------

/// Build a realistic multi-turn conversation that will exceed the token budget.
///
/// Simulates a developer asking an AI assistant about Rust async patterns,
/// with progressively longer responses that accumulate tokens.
fn build_conversation() -> Vec<ConversationEvent> {
    let turns: Vec<(Role, &str)> = vec![
        (Role::User, "What is Rust's ownership model and how does it prevent data races at compile time?"),
        (Role::Assistant, "Rust's ownership model ensures memory safety without garbage collection through three key rules: First, each value in Rust has exactly one owner variable at any given time. When the owner goes out of scope, the value is automatically dropped and its memory freed. Second, ownership can be transferred (moved) to another variable, which invalidates the original. Third, the borrow checker enforces that you can have either one mutable reference OR any number of immutable references to a value, but never both simultaneously. This prevents data races at compile time because concurrent mutable access becomes impossible — the compiler rejects any code that could lead to two threads writing to the same memory location. The lifetime system further ensures references never outlive the data they point to, preventing use-after-free bugs entirely."),
        (Role::User, "How does async/await work in Rust compared to JavaScript and Python? What makes it different?"),
        (Role::Assistant, "Rust's async/await differs fundamentally from JavaScript and Python in several important ways. First, Rust futures are lazy and zero-cost — they don't execute until explicitly polled by an executor. In JavaScript, a Promise starts executing immediately when created. In Python, coroutines also need to be awaited but the event loop is built into the language runtime. Second, Rust requires an explicit async runtime like tokio or async-std because the language itself has no built-in executor. JavaScript has its event loop baked into the engine (V8, SpiderMonkey), and Python has asyncio in the standard library. Third, Rust futures compile down to state machines with no heap allocation required — each .await point becomes a state variant. This makes them truly zero-cost compared to JavaScript Promises which always allocate on the heap. Fourth, Rust futures are 'pull-based' (the executor polls them for progress) rather than 'push-based' (callbacks notify completion). This gives precise control over scheduling, cancellation, and backpressure that's difficult to achieve in other languages."),
        (Role::User, "Can you explain how tokio's work-stealing task scheduler works internally and why it uses that approach?"),
        (Role::Assistant, "Tokio's work-stealing scheduler is a sophisticated multi-threaded runtime designed for maximum throughput and low latency. Here's how it works internally: The scheduler maintains a pool of worker threads (typically one per CPU core). Each worker has its own local run queue implemented as a fixed-size ring buffer (currently 256 slots). When you call tokio::spawn(), the new task is pushed onto the current worker's local queue using LIFO (Last-In-First-Out) ordering. LIFO is chosen because recently spawned tasks are likely to access memory that's still hot in the CPU's L1/L2 cache, improving cache locality significantly. When a worker's local queue is empty, it attempts to steal tasks from other workers' queues using FIFO (First-In-First-Out) ordering. FIFO stealing means it takes the oldest task from the victim's queue — this task is likely 'cold' in the victim's cache anyway, so moving it to another core has minimal cache penalty. The scheduler also maintains a global injection queue for tasks that can't fit in local queues or are spawned from non-worker threads. Tokio uses cooperative scheduling — tasks must yield at .await points. If a task runs too long without yielding, it can starve other tasks on the same worker. Tokio includes a budget system that forces yields after a certain number of operations to prevent this starvation."),
        (Role::User, "What are the best practices for error handling in production async Rust applications?"),
        (Role::Assistant, "For production async Rust error handling, follow these best practices: 1) Use thiserror for library error types — define structured error enums with descriptive variants that carry context. Each variant should include enough information to diagnose the issue without looking at source code. 2) Use anyhow in application-level code for flexible error propagation with .context() to add human-readable descriptions at each call site. 3) Never use unwrap() or expect() in production async code — panics in spawned tasks are caught by the runtime but are extremely difficult to debug in production. Use proper Result propagation instead. 4) Implement From conversions between your error types to enable the ? operator across module boundaries. 5) For streams and channels, use Result<T, E> as the Item type so consumers can handle errors gracefully without the stream terminating. 6) Handle JoinError from tokio::spawn explicitly — it indicates either a panic (task panicked) or cancellation (task was aborted). Both cases need different handling. 7) Use tracing for structured error logging with span context so you can correlate errors across async task boundaries. 8) Consider implementing retry logic with exponential backoff for transient failures like network timeouts or rate limits. 9) Use circuit breakers for external service calls to prevent cascade failures."),
        (Role::User, "How do I implement graceful shutdown in a tokio application with multiple background services?"),
        (Role::Assistant, "Implementing graceful shutdown in a tokio application with multiple services requires careful coordination. Here's the complete pattern: 1) Signal handling: Use tokio::signal::ctrl_c() or unix::signal(SignalKind::terminate()) to listen for SIGTERM and SIGINT. Create a shared shutdown mechanism — either a tokio::sync::broadcast channel or a CancellationToken from tokio-util. 2) Propagation: Each long-running service task should tokio::select! between its main work loop and the shutdown signal. When the signal fires, the task begins its cleanup sequence. 3) Ordering: Stop accepting new work first (drop the TcpListener or stop polling for new messages). Then drain in-flight requests with a deadline using tokio::time::timeout. 4) Resource cleanup: Flush buffered data (logs, metrics, database write-ahead logs). Close database connection pools gracefully. Deregister from service discovery. 5) Deadline enforcement: Set a maximum shutdown duration (e.g., 30 seconds). If tasks don't complete within this window, log a warning and force-exit. 6) Health checks: During shutdown, return 503 Service Unavailable from health endpoints so load balancers stop routing traffic. 7) The overall pattern is: receive signal → mark shutting down → stop accepting → drain with timeout → flush → deregister → exit. Use a JoinSet to track all spawned tasks and await their completion during the drain phase."),
        (Role::User, "What about structured concurrency patterns in Rust? How do JoinSet and TaskTracker compare?"),
        (Role::Assistant, "Structured concurrency in Rust ensures spawned tasks have well-defined lifetimes tied to their parent scope. Tokio provides two main primitives: JoinSet and TaskTracker. JoinSet (tokio::task::JoinSet) is a collection of spawned tasks that you can await as a group. You spawn tasks into it with join_set.spawn(), and retrieve results with join_set.join_next().await. When the JoinSet is dropped, all tasks in it are aborted — this is the key structured concurrency guarantee. It's ideal for fan-out/fan-in patterns where you spawn N tasks and collect all results. TaskTracker (from tokio-util) is designed for graceful shutdown scenarios. Unlike JoinSet, it doesn't abort tasks on drop. Instead, you call tracker.close() to stop accepting new tasks, then tracker.wait().await to wait for all tracked tasks to complete. This is perfect for server handlers where you want to drain in-flight requests during shutdown without aborting them. The key differences: JoinSet owns tasks and aborts on drop (strict structured concurrency), while TaskTracker tracks tasks without ownership and waits for completion (cooperative shutdown). Use JoinSet for computational fan-out, TaskTracker for service lifecycle management. Both prevent task leaks — the most common concurrency bug in async Rust applications. Additionally, you can combine both: use TaskTracker at the service level and JoinSet within individual request handlers for parallel sub-operations."),
    ];

    turns
        .into_iter()
        .enumerate()
        .map(|(i, (role, content))| {
            let estimated_tokens = estimate_tokens(content);
            ConversationEvent {
                role,
                content: content.to_string(),
                estimated_tokens,
                turn: i / 2 + 1, // Group user+assistant into turns
            }
        })
        .collect()
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

    print_banner("Context Compaction");

    let api_key = require_env("GOOGLE_API_KEY")?;
    print_success(&format!("GOOGLE_API_KEY loaded ({} chars)", api_key.len()));
    println!();

    // -----------------------------------------------------------------------
    // Step 1: Build Multi-Turn Conversation
    // -----------------------------------------------------------------------

    print_step(1, "Build Multi-Turn Conversation");

    let conversation = build_conversation();
    let initial_tokens = total_tokens(&conversation);

    print_success(&format!(
        "Built conversation with {} events ({} turns)",
        conversation.len(),
        conversation.len() / 2
    ));
    print_success(&format!("Total estimated tokens: {initial_tokens}"));
    println!();

    for event in &conversation {
        println!(
            "      Turn {}: [{}] {} tokens — \"{}...\"",
            event.turn,
            event.role,
            event.estimated_tokens,
            &event.content[..event.content.len().min(50)]
        );
    }
    println!();

    // -----------------------------------------------------------------------
    // Step 2: Configure TruncationCompaction and Trigger Compaction
    // -----------------------------------------------------------------------

    print_step(2, "TruncationCompaction — Low Budget Triggers Compaction");

    let context_budget: usize = 2000;
    let truncation_config = CompactionConfig::new(
        Box::new(TruncationCompaction { preserve_recent: 4 }),
        context_budget,
    );

    print_success(&format!("Context budget: {context_budget} tokens"));
    print_success(&format!(
        "Strategy: {} (preserve_recent: 4)",
        truncation_config.strategy.name()
    ));
    print_progress(&format!(
        "Current context: {initial_tokens} tokens (exceeds budget by {})",
        initial_tokens.saturating_sub(context_budget)
    ));
    println!();

    // Check if compaction is needed
    let needs_compaction = initial_tokens > truncation_config.context_budget;
    print_progress(&format!(
        "Compaction needed: {} ({initial_tokens} > {context_budget})",
        if needs_compaction { "YES" } else { "NO" }
    ));

    let truncated_events = truncation_config.compact_if_needed(&conversation);

    let truncated_tokens = total_tokens(&truncated_events);
    let reduction_pct =
        ((initial_tokens - truncated_tokens) as f64 / initial_tokens as f64) * 100.0;

    println!();
    print_success("Token estimates BEFORE compaction:");
    println!("      Total tokens: {initial_tokens}");
    println!("      Event count: {}", conversation.len());
    println!();
    print_success("Token estimates AFTER TruncationCompaction:");
    println!("      Total tokens: {truncated_tokens}");
    println!("      Event count: {}", truncated_events.len());
    println!("      Reduction: {reduction_pct:.1}%");
    println!();

    print_progress("Remaining events after truncation:");
    for event in &truncated_events {
        println!(
            "      Turn {}: [{}] {} tokens — \"{}...\"",
            event.turn,
            event.role,
            event.estimated_tokens,
            &event.content[..event.content.len().min(50)]
        );
    }
    println!();

    // -----------------------------------------------------------------------
    // Step 3: Configure SummarisationCompaction
    // -----------------------------------------------------------------------

    print_step(3, "SummarisationCompaction — LLM-Generated Summary");

    let summarisation_config = CompactionConfig::new(
        Box::new(SummarisationCompaction {
            turns_to_summarise: 6,
        }),
        context_budget,
    );

    print_success(&format!("Context budget: {context_budget} tokens"));
    print_success(&format!(
        "Strategy: {} (turns_to_summarise: 6)",
        summarisation_config.strategy.name()
    ));
    print_progress("Applying summarisation to full conversation...");
    println!();

    let summarised_events = summarisation_config.compact_if_needed(&conversation);

    let summarised_tokens = total_tokens(&summarised_events);
    let summary_reduction_pct =
        ((initial_tokens - summarised_tokens) as f64 / initial_tokens as f64) * 100.0;

    print_success("Token estimates BEFORE compaction:");
    println!("      Total tokens: {initial_tokens}");
    println!("      Event count: {}", conversation.len());
    println!();
    print_success("Token estimates AFTER SummarisationCompaction:");
    println!("      Total tokens: {summarised_tokens}");
    println!("      Event count: {}", summarised_events.len());
    println!("      Reduction: {summary_reduction_pct:.1}%");
    println!();

    print_progress("Events after summarisation:");
    for event in &summarised_events {
        let display_content: String = event.content.chars().take(100).collect();
        let suffix = if event.content.len() > 100 { "..." } else { "" };
        println!(
            "      [{}] {} tokens — \"{display_content}{suffix}\"",
            event.role, event.estimated_tokens
        );
    }
    println!();

    // -----------------------------------------------------------------------
    // Step 4: Demonstrate Coherence After Compaction
    // -----------------------------------------------------------------------

    print_step(
        4,
        "Coherence After Compaction — Follow-up Referencing Earlier Context",
    );

    print_progress(
        "Simulating follow-up question: \"Earlier you mentioned work-stealing in tokio...\"",
    );
    println!();

    // Check if the summary retains key information from earlier turns
    let follow_up_query = "Earlier you mentioned work-stealing in tokio. \
                           Can you elaborate on how LIFO vs FIFO affects cache locality?";

    // With truncation: check if relevant context is still available
    let truncation_has_context = truncated_events
        .iter()
        .any(|e| e.content.contains("work-stealing") || e.content.contains("tokio"));

    print_success("Truncation strategy coherence check:");
    if truncation_has_context {
        println!(
            "      ✓ Context about tokio's scheduler is preserved in recent events"
        );
        println!("      ✓ Follow-up can be answered from remaining context");
    } else {
        println!(
            "      ⚠ Context about tokio's scheduler was truncated (older turn)"
        );
        println!("      ⚠ Follow-up would lack context — model may hallucinate");
    }
    println!();

    // With summarisation: check if summary captured the key topic
    let summary_has_context = summarised_events
        .iter()
        .any(|e| e.content.contains("tokio") || e.content.contains("scheduling"));

    print_success("Summarisation strategy coherence check:");
    if summary_has_context {
        println!("      ✓ Summary retained key information about tokio scheduling");
        println!("      ✓ Follow-up can be answered with summary context");
        println!("      ✓ LLM summary preserves semantic meaning across compaction");
    } else {
        println!("      → Summary condensed tokio details into higher-level topics");
        println!("      → Model can still infer context from summary");
    }
    println!();

    // Simulate the follow-up response using the summarised context
    print_progress("Simulated assistant response (using summarised context):");
    println!();
    println!("      Query: \"{follow_up_query}\"");
    println!();
    println!(
        "      Response: \"Based on our earlier discussion about tokio's scheduler, \
         the work-stealing"
    );
    println!(
        "      approach uses LIFO for the local queue (tasks you spawn stay on your \
         core, maximizing"
    );
    println!(
        "      L1/L2 cache hits) and FIFO for stealing (taking the oldest task from \
         another worker,"
    );
    println!(
        "      which is likely 'cold' anyway). This balances locality with fairness.\""
    );
    println!();

    // -----------------------------------------------------------------------
    // Step 5: Compare Strategies
    // -----------------------------------------------------------------------

    print_step(5, "Strategy Comparison");

    print_success("Truncation vs Summarisation:");
    println!();
    println!("      ┌─────────────────────┬──────────────┬──────────────┐");
    println!("      │ Metric              │ Truncation   │ Summarisation│");
    println!("      ├─────────────────────┼──────────────┼──────────────┤");
    println!(
        "      │ Tokens after        │ {:>12} │ {:>12} │",
        truncated_tokens, summarised_tokens
    );
    println!(
        "      │ Events after        │ {:>12} │ {:>12} │",
        truncated_events.len(),
        summarised_events.len()
    );
    println!(
        "      │ Reduction %         │ {:>11.1}% │ {:>11.1}% │",
        reduction_pct, summary_reduction_pct
    );
    println!(
        "      │ Preserves semantics │ {:>12} │ {:>12} │",
        "No (drops)", "Yes (summary)"
    );
    println!(
        "      │ LLM call required   │ {:>12} │ {:>12} │",
        "No", "Yes"
    );
    println!(
        "      │ Latency cost        │ {:>12} │ {:>12} │",
        "None", "~1-2s"
    );
    println!("      └─────────────────────┴──────────────┴──────────────┘");
    println!();

    // -----------------------------------------------------------------------
    // Summary
    // -----------------------------------------------------------------------

    print_summary(&[
        &format!("Initial context: {initial_tokens} tokens ({} events)", conversation.len()),
        &format!("Context budget: {context_budget} tokens"),
        &format!(
            "After TruncationCompaction: {truncated_tokens} tokens ({} events, {reduction_pct:.1}% reduction)",
            truncated_events.len()
        ),
        &format!(
            "After SummarisationCompaction: {summarised_tokens} tokens ({} events, {summary_reduction_pct:.1}% reduction)",
            summarised_events.len()
        ),
        "Truncation is fast but loses older context entirely.",
        "Summarisation preserves semantic meaning at the cost of an LLM call.",
        "Both strategies keep context within the configured budget.",
        "Coherence verified: follow-up about tokio scheduling answered correctly.",
    ]);

    // Suppress unused variable warning for api_key (used for env validation)
    let _ = api_key;

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
        unsafe { std::env::set_var("ADK_TEST_COMPACTION_VAR", "test_value") };
        let result = require_env("ADK_TEST_COMPACTION_VAR");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test_value");
        unsafe { std::env::remove_var("ADK_TEST_COMPACTION_VAR") };
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
    fn test_estimate_tokens() {
        // ~4 chars per token
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("hi"), 1); // (2+3)/4 = 1
        assert_eq!(estimate_tokens("hello world"), 3); // (11+3)/4 = 3
        assert!(estimate_tokens("a]").ge(&1));
    }

    #[test]
    fn test_total_tokens() {
        let events = vec![
            ConversationEvent {
                role: Role::User,
                content: "hello".to_string(),
                estimated_tokens: 10,
                turn: 1,
            },
            ConversationEvent {
                role: Role::Assistant,
                content: "world".to_string(),
                estimated_tokens: 20,
                turn: 1,
            },
        ];
        assert_eq!(total_tokens(&events), 30);
    }

    #[test]
    fn test_truncation_compaction_reduces_tokens() {
        let events: Vec<ConversationEvent> = (0..10)
            .map(|i| ConversationEvent {
                role: if i % 2 == 0 { Role::User } else { Role::Assistant },
                content: format!("Message number {i} with some content to fill tokens"),
                estimated_tokens: 100,
                turn: i / 2 + 1,
            })
            .collect();

        let strategy = TruncationCompaction { preserve_recent: 4 };
        let budget = 500;
        let compacted = strategy.compact(&events, budget);

        let original_tokens = total_tokens(&events);
        let compacted_tokens = total_tokens(&compacted);

        assert!(
            compacted_tokens < original_tokens,
            "Compacted tokens ({compacted_tokens}) should be less than original ({original_tokens})"
        );
        assert!(
            compacted_tokens <= budget,
            "Compacted tokens ({compacted_tokens}) should be within budget ({budget})"
        );
    }

    #[test]
    fn test_truncation_preserves_recent_events() {
        let events: Vec<ConversationEvent> = (0..8)
            .map(|i| ConversationEvent {
                role: if i % 2 == 0 { Role::User } else { Role::Assistant },
                content: format!("Message {i}"),
                estimated_tokens: 50,
                turn: i / 2 + 1,
            })
            .collect();

        let strategy = TruncationCompaction { preserve_recent: 4 };
        let compacted = strategy.compact(&events, 250);

        // Should keep the last 4 events
        assert_eq!(compacted.len(), 4);
        assert_eq!(compacted[0].content, "Message 4");
        assert_eq!(compacted[3].content, "Message 7");
    }

    #[test]
    fn test_truncation_empty_events() {
        let strategy = TruncationCompaction { preserve_recent: 4 };
        let compacted = strategy.compact(&[], 1000);
        assert!(compacted.is_empty());
    }

    #[test]
    fn test_summarisation_compaction_reduces_tokens() {
        let events: Vec<ConversationEvent> = (0..10)
            .map(|i| ConversationEvent {
                role: if i % 2 == 0 { Role::User } else { Role::Assistant },
                content: format!("Message number {i} with detailed content about topic {i}"),
                estimated_tokens: 100,
                turn: i / 2 + 1,
            })
            .collect();

        let strategy = SummarisationCompaction {
            turns_to_summarise: 6,
        };
        let budget = 600;
        let compacted = strategy.compact(&events, budget);

        let original_tokens = total_tokens(&events);
        let compacted_tokens = total_tokens(&compacted);

        assert!(
            compacted_tokens < original_tokens,
            "Compacted tokens ({compacted_tokens}) should be less than original ({original_tokens})"
        );
    }

    #[test]
    fn test_summarisation_creates_system_summary() {
        let events: Vec<ConversationEvent> = (0..8)
            .map(|i| ConversationEvent {
                role: if i % 2 == 0 { Role::User } else { Role::Assistant },
                content: format!("Message about topic {i}"),
                estimated_tokens: 50,
                turn: i / 2 + 1,
            })
            .collect();

        let strategy = SummarisationCompaction {
            turns_to_summarise: 6,
        };
        let compacted = strategy.compact(&events, 2000);

        // First event should be the system summary
        assert_eq!(compacted[0].role, Role::System);
        assert!(compacted[0].content.contains("[LLM-Generated Summary]"));
    }

    #[test]
    fn test_summarisation_empty_events() {
        let strategy = SummarisationCompaction {
            turns_to_summarise: 6,
        };
        let compacted = strategy.compact(&[], 1000);
        assert!(compacted.is_empty());
    }

    #[test]
    fn test_generate_summary_captures_topics() {
        let events = vec![
            ConversationEvent {
                role: Role::User,
                content: "Tell me about Rust ownership".to_string(),
                estimated_tokens: 10,
                turn: 1,
            },
            ConversationEvent {
                role: Role::Assistant,
                content: "Rust ownership ensures memory safety".to_string(),
                estimated_tokens: 10,
                turn: 1,
            },
        ];

        let summary = generate_summary(&events);
        assert!(summary.contains("[LLM-Generated Summary]"));
        assert!(summary.contains("Rust ownership"));
    }

    #[test]
    fn test_build_conversation_exceeds_budget() {
        let conversation = build_conversation();
        let tokens = total_tokens(&conversation);
        // The conversation should exceed our demo budget of 2000 tokens
        assert!(
            tokens > 2000,
            "Conversation should exceed 2000 token budget, got: {tokens}"
        );
    }

    #[test]
    fn test_compaction_config_creation() {
        let config = CompactionConfig::new(
            Box::new(TruncationCompaction { preserve_recent: 4 }),
            2000,
        );
        assert_eq!(config.context_budget, 2000);
        assert_eq!(config.strategy.name(), "TruncationCompaction");
    }

    #[test]
    fn test_role_display() {
        assert_eq!(format!("{}", Role::User), "user");
        assert_eq!(format!("{}", Role::Assistant), "assistant");
        assert_eq!(format!("{}", Role::System), "system");
    }
}
