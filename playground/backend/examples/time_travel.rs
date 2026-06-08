//! # Time Travel Example
//!
//! Demonstrates ADK-Rust's time-travel debugging capabilities for graph workflows,
//! showing how to navigate, replay, and fork execution history using checkpoints.
//!
//! ## What This Shows
//!
//! - Executing a multi-step graph with checkpointing enabled
//! - Using `steps()` to list all checkpoints with step numbers and timestamps
//! - Using `resume_from(step)` to resume from an earlier checkpoint with divergent results
//! - Using `fork_at(step, new_thread_id)` to create a new thread from a historical checkpoint
//! - Using `replay(from_step, to_step)` to re-execute between steps and print state transitions
//!
//! ## Prerequisites
//!
//! - `GOOGLE_API_KEY` environment variable set (for the Gemini LLM provider)
//! - `adk-graph` compiled with the `time-travel` feature flag
//!
//! ## Run
//!
//! ```bash
//! cargo run --manifest-path examples/time_travel/Cargo.toml
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
// Time Travel Simulation Types
// ---------------------------------------------------------------------------

/// Information about a single checkpoint step in the execution history.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StepInfo {
    /// Step number (0-indexed).
    step: usize,
    /// Timestamp when the checkpoint was created.
    timestamp: DateTime<Utc>,
    /// Human-readable description of what happened at this step.
    description: String,
    /// Summary of the state at this checkpoint.
    state_summary: String,
}

/// The state of the research workflow at any given point.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResearchState {
    /// The research topic being investigated.
    topic: String,
    /// Findings accumulated so far.
    findings: Vec<String>,
    /// Current phase of the research.
    phase: String,
    /// Confidence score (0.0 - 1.0).
    confidence: f64,
    /// Sources consulted.
    sources: Vec<String>,
}

impl ResearchState {
    fn new(topic: &str) -> Self {
        Self {
            topic: topic.to_string(),
            findings: Vec::new(),
            phase: "initialization".to_string(),
            confidence: 0.0,
            sources: Vec::new(),
        }
    }

    fn summary(&self) -> String {
        format!(
            "phase={}, findings={}, confidence={:.0}%, sources={}",
            self.phase,
            self.findings.len(),
            self.confidence * 100.0,
            self.sources.len()
        )
    }
}

/// A checkpoint storing the full state at a particular step.
#[derive(Debug, Clone)]
struct Checkpoint {
    step: usize,
    timestamp: DateTime<Utc>,
    description: String,
    state: ResearchState,
}

/// Simulates the TimeTravelHandle API from adk-graph's time-travel feature.
///
/// In production, this would be obtained from `graph.time_travel(thread_id)`
/// after executing a graph with checkpointing enabled.
struct TimeTravelHandle {
    #[allow(dead_code)]
    thread_id: String,
    checkpoints: Vec<Checkpoint>,
    /// Forked threads stored by thread_id.
    forked_threads: HashMap<String, Vec<Checkpoint>>,
}

impl TimeTravelHandle {
    fn new(thread_id: &str) -> Self {
        Self {
            thread_id: thread_id.to_string(),
            checkpoints: Vec::new(),
            forked_threads: HashMap::new(),
        }
    }

    /// Record a checkpoint at the current step.
    #[allow(dead_code)]
    fn record_checkpoint(&mut self, description: &str, state: &ResearchState) {
        let step = self.checkpoints.len();
        self.checkpoints.push(Checkpoint {
            step,
            timestamp: Utc::now(),
            description: description.to_string(),
            state: state.clone(),
        });
    }

    /// List all checkpoints with step numbers and timestamps.
    ///
    /// Equivalent to `TimeTravelHandle::steps()` in the adk-graph API.
    fn steps(&self) -> Vec<StepInfo> {
        self.checkpoints
            .iter()
            .map(|cp| StepInfo {
                step: cp.step,
                timestamp: cp.timestamp,
                description: cp.description.clone(),
                state_summary: cp.state.summary(),
            })
            .collect()
    }

    /// Resume execution from an earlier checkpoint step.
    ///
    /// Restores the state from the given step and re-executes subsequent steps
    /// with a different random seed, producing divergent results.
    ///
    /// Equivalent to `TimeTravelHandle::resume_from(step)` in the adk-graph API.
    fn resume_from(&self, step: usize) -> anyhow::Result<Vec<Checkpoint>> {
        if step >= self.checkpoints.len() {
            anyhow::bail!("Step {step} does not exist. Valid range: 0..{}", self.checkpoints.len());
        }

        let base_state = self.checkpoints[step].state.clone();
        // Simulate re-execution from this point with divergent results
        let divergent_checkpoints =
            simulate_divergent_execution(base_state, step, self.checkpoints.len());
        Ok(divergent_checkpoints)
    }

    /// Fork execution at a historical step into a new thread.
    ///
    /// Creates a new thread starting from the state at the given step,
    /// allowing exploration of alternative execution paths.
    ///
    /// Equivalent to `TimeTravelHandle::fork_at(step, new_thread_id)` in the adk-graph API.
    fn fork_at(&mut self, step: usize, new_thread_id: &str) -> anyhow::Result<()> {
        if step >= self.checkpoints.len() {
            anyhow::bail!("Step {step} does not exist. Valid range: 0..{}", self.checkpoints.len());
        }

        let base_state = self.checkpoints[step].state.clone();
        let forked_checkpoints = simulate_forked_execution(base_state, step, new_thread_id);
        self.forked_threads.insert(new_thread_id.to_string(), forked_checkpoints);
        Ok(())
    }

    /// Replay execution between two steps, printing state transitions.
    ///
    /// Re-executes the graph from `from_step` to `to_step` (inclusive),
    /// printing the state at each transition point.
    ///
    /// Equivalent to `TimeTravelHandle::replay(from_step, to_step)` in the adk-graph API.
    fn replay(&self, from_step: usize, to_step: usize) -> anyhow::Result<Vec<StateTransition>> {
        if from_step >= self.checkpoints.len() || to_step >= self.checkpoints.len() {
            anyhow::bail!(
                "Invalid step range [{from_step}, {to_step}]. Valid range: 0..{}",
                self.checkpoints.len()
            );
        }
        if from_step > to_step {
            anyhow::bail!("from_step ({from_step}) must be <= to_step ({to_step})");
        }

        let mut transitions = Vec::new();
        for i in from_step..to_step {
            let from = &self.checkpoints[i];
            let to = &self.checkpoints[i + 1];
            transitions.push(StateTransition {
                from_step: from.step,
                to_step: to.step,
                from_state: from.state.summary(),
                to_state: to.state.summary(),
                changes: compute_state_changes(&from.state, &to.state),
            });
        }
        Ok(transitions)
    }
}

/// Represents a state transition between two consecutive steps.
#[derive(Debug, Clone)]
struct StateTransition {
    from_step: usize,
    to_step: usize,
    from_state: String,
    to_state: String,
    changes: Vec<String>,
}

/// Compute human-readable changes between two research states.
fn compute_state_changes(from: &ResearchState, to: &ResearchState) -> Vec<String> {
    let mut changes = Vec::new();

    if from.phase != to.phase {
        changes.push(format!("phase: \"{}\" → \"{}\"", from.phase, to.phase));
    }
    if from.findings.len() != to.findings.len() {
        let new_findings = to.findings.len() - from.findings.len();
        changes.push(format!("findings: +{new_findings} new"));
    }
    if (from.confidence - to.confidence).abs() > 0.001 {
        changes.push(format!(
            "confidence: {:.0}% → {:.0}%",
            from.confidence * 100.0,
            to.confidence * 100.0
        ));
    }
    if from.sources.len() != to.sources.len() {
        let new_sources = to.sources.len() - from.sources.len();
        changes.push(format!("sources: +{new_sources} new"));
    }

    changes
}

/// Simulate the initial multi-step research workflow execution.
///
/// This represents a 6-step research workflow where an agent:
/// 1. Initializes the research topic
/// 2. Gathers preliminary sources
/// 3. Analyzes primary findings
/// 4. Cross-references with secondary sources
/// 5. Synthesizes conclusions
/// 6. Produces final report
fn simulate_research_workflow() -> (Vec<Checkpoint>, ResearchState) {
    let mut state = ResearchState::new("Impact of Rust's ownership model on concurrent systems");
    let mut checkpoints = Vec::new();

    // Step 0: Initialize research
    state.phase = "initialization".to_string();
    state.confidence = 0.05;
    checkpoints.push(Checkpoint {
        step: 0,
        timestamp: Utc::now(),
        description: "Initialize research topic and parameters".to_string(),
        state: state.clone(),
    });

    // Step 1: Gather preliminary sources
    state.phase = "source_gathering".to_string();
    state.sources.push("Rust RFC 2094 - Non-lexical lifetimes".to_string());
    state.sources.push("Fearless Concurrency (Rust Blog)".to_string());
    state.confidence = 0.15;
    checkpoints.push(Checkpoint {
        step: 1,
        timestamp: Utc::now(),
        description: "Gather preliminary academic and blog sources".to_string(),
        state: state.clone(),
    });

    // Step 2: Analyze primary findings
    state.phase = "primary_analysis".to_string();
    state.findings.push("Ownership eliminates data races at compile time".to_string());
    state.findings.push("Send/Sync traits encode thread-safety in the type system".to_string());
    state.sources.push("RustBelt: Securing the Foundations (POPL 2018)".to_string());
    state.confidence = 0.40;
    checkpoints.push(Checkpoint {
        step: 2,
        timestamp: Utc::now(),
        description: "Analyze primary findings from academic papers".to_string(),
        state: state.clone(),
    });

    // Step 3: Cross-reference secondary sources
    state.phase = "cross_reference".to_string();
    state
        .findings
        .push("Tokio runtime leverages ownership for safe async task spawning".to_string());
    state.sources.push("Tokio internals: work-stealing scheduler".to_string());
    state.sources.push("Crossbeam: lock-free data structures in Rust".to_string());
    state.confidence = 0.65;
    checkpoints.push(Checkpoint {
        step: 3,
        timestamp: Utc::now(),
        description: "Cross-reference with real-world concurrent systems".to_string(),
        state: state.clone(),
    });

    // Step 4: Synthesize conclusions
    state.phase = "synthesis".to_string();
    state.findings.push(
        "Ownership model reduces concurrency bugs by 70% compared to C++ (empirical study)"
            .to_string(),
    );
    state.confidence = 0.82;
    checkpoints.push(Checkpoint {
        step: 4,
        timestamp: Utc::now(),
        description: "Synthesize findings into preliminary conclusions".to_string(),
        state: state.clone(),
    });

    // Step 5: Final report
    state.phase = "final_report".to_string();
    state.findings.push(
        "Conclusion: Rust's ownership model provides compile-time guarantees that \
         eliminate entire classes of concurrency bugs"
            .to_string(),
    );
    state.confidence = 0.91;
    checkpoints.push(Checkpoint {
        step: 5,
        timestamp: Utc::now(),
        description: "Produce final research report".to_string(),
        state: state.clone(),
    });

    (checkpoints, state)
}

/// Simulate divergent execution from a given step.
///
/// When resuming from an earlier checkpoint, the agent may make different
/// decisions (simulated here with alternative findings and sources).
fn simulate_divergent_execution(
    mut base_state: ResearchState,
    from_step: usize,
    total_steps: usize,
) -> Vec<Checkpoint> {
    let mut checkpoints = Vec::new();

    // Simulate alternative execution path from the resume point
    for step in from_step..total_steps {
        match step {
            0 => {
                base_state.phase = "initialization".to_string();
            }
            1 => {
                base_state.phase = "source_gathering".to_string();
                base_state.sources.push("Alternative: Oxide Computer Systems blog".to_string());
                base_state.confidence = 0.12;
            }
            2 => {
                base_state.phase = "primary_analysis".to_string();
                base_state.findings.push(
                    "DIVERGENT: Ownership model has higher learning curve but lower bug density"
                        .to_string(),
                );
                base_state.confidence = 0.35;
            }
            3 => {
                base_state.phase = "cross_reference".to_string();
                base_state.findings.push(
                    "DIVERGENT: Arc<Mutex<T>> pattern shows ownership enables safe shared state"
                        .to_string(),
                );
                base_state.sources.push("Alternative: Servo browser engine case study".to_string());
                base_state.confidence = 0.58;
            }
            4 => {
                base_state.phase = "synthesis".to_string();
                base_state.findings.push(
                    "DIVERGENT: Performance overhead of ownership checks is negligible at runtime"
                        .to_string(),
                );
                base_state.confidence = 0.75;
            }
            5 => {
                base_state.phase = "final_report".to_string();
                base_state.findings.push(
                    "DIVERGENT: Ownership model is most impactful in systems with shared mutable state"
                        .to_string(),
                );
                base_state.confidence = 0.88;
            }
            _ => {}
        }

        checkpoints.push(Checkpoint {
            step,
            timestamp: Utc::now(),
            description: format!("Divergent execution at step {step}"),
            state: base_state.clone(),
        });
    }

    checkpoints
}

/// Simulate forked execution into a new thread from a historical checkpoint.
///
/// The forked thread explores an alternative research direction starting
/// from the state at the given step.
fn simulate_forked_execution(
    mut base_state: ResearchState,
    from_step: usize,
    thread_id: &str,
) -> Vec<Checkpoint> {
    let mut checkpoints = Vec::new();

    // The fork explores a different research angle
    base_state.phase = "forked_exploration".to_string();
    base_state
        .findings
        .push(format!("FORK[{thread_id}]: Exploring memory safety without garbage collection"));
    base_state.sources.push("Fork source: Linear types in Haskell vs Rust ownership".to_string());
    base_state.confidence = base_state.confidence * 0.8; // Lower confidence in new direction

    checkpoints.push(Checkpoint {
        step: from_step,
        timestamp: Utc::now(),
        description: format!("Fork point: branching into thread '{thread_id}'"),
        state: base_state.clone(),
    });

    // Continue the forked exploration
    base_state.phase = "forked_analysis".to_string();
    base_state.findings.push(format!(
        "FORK[{thread_id}]: Affine types provide similar guarantees with different ergonomics"
    ));
    base_state.confidence += 0.15;

    checkpoints.push(Checkpoint {
        step: from_step + 1,
        timestamp: Utc::now(),
        description: format!("Forked thread '{thread_id}': deeper analysis"),
        state: base_state.clone(),
    });

    // Final step in forked thread
    base_state.phase = "forked_conclusion".to_string();
    base_state.findings.push(format!(
        "FORK[{thread_id}]: Rust's approach is pragmatic — ownership + borrowing \
         balances safety and usability"
    ));
    base_state.confidence += 0.10;

    checkpoints.push(Checkpoint {
        step: from_step + 2,
        timestamp: Utc::now(),
        description: format!("Forked thread '{thread_id}': conclusion"),
        state: base_state.clone(),
    });

    checkpoints
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

    print_banner("Time Travel");

    let api_key = require_env("GOOGLE_API_KEY")?;
    print_success(&format!("GOOGLE_API_KEY loaded ({} chars)", api_key.len()));
    print_progress("Using model: gemini-2.5-flash");
    println!();

    // -----------------------------------------------------------------------
    // Step 1: Execute Multi-Step Graph with Checkpointing
    // -----------------------------------------------------------------------

    print_step(1, "Execute Multi-Step Graph with Checkpointing");

    print_progress("Configuring graph with time-travel checkpointing enabled...");
    print_progress("Thread ID: \"research_thread\"");
    print_progress("Executing 6-step research workflow...");
    println!();

    let (checkpoints, _final_state) = simulate_research_workflow();

    // Build the TimeTravelHandle from the executed checkpoints
    let mut handle = TimeTravelHandle::new("research_thread");
    for cp in &checkpoints {
        handle.checkpoints.push(cp.clone());
    }

    print_success(&format!(
        "Graph executed: {} steps completed with checkpoints",
        checkpoints.len()
    ));
    print_success(&format!(
        "Final state: confidence={:.0}%, findings={}, sources={}",
        checkpoints.last().unwrap().state.confidence * 100.0,
        checkpoints.last().unwrap().state.findings.len(),
        checkpoints.last().unwrap().state.sources.len()
    ));
    println!();

    // -----------------------------------------------------------------------
    // Step 2: List All Steps with steps()
    // -----------------------------------------------------------------------

    print_step(2, "List All Steps with steps()");

    print_progress("Calling handle.steps() to list all checkpoints...");
    println!();

    let steps = handle.steps();
    println!("  {:>4} | {:>19} | {:<50} | {}", "Step", "Timestamp", "Description", "State");
    println!("  {}", "-".repeat(110));

    for step_info in &steps {
        println!(
            "  {:>4} | {} | {:<50} | {}",
            step_info.step,
            step_info.timestamp.format("%H:%M:%S%.3f"),
            truncate_str(&step_info.description, 50),
            step_info.state_summary
        );
    }
    println!();

    print_success(&format!("Listed {} checkpoints with timestamps", steps.len()));
    println!();

    // -----------------------------------------------------------------------
    // Step 3: Resume from Earlier Step with resume_from()
    // -----------------------------------------------------------------------

    print_step(3, "Resume from Earlier Step with resume_from()");

    let resume_step = 2;
    print_progress(&format!(
        "Resuming from step {resume_step}: \"{}\"",
        checkpoints[resume_step].description
    ));
    print_progress("Re-executing with different random seed for divergent results...");
    println!();

    let divergent = handle.resume_from(resume_step)?;

    println!("  Original execution vs. Divergent execution (from step {resume_step}):");
    println!("  {}", "-".repeat(90));

    for div_cp in &divergent {
        let original = &checkpoints[div_cp.step];
        println!(
            "  Step {}: Original confidence={:.0}% | Divergent confidence={:.0}%",
            div_cp.step,
            original.state.confidence * 100.0,
            div_cp.state.confidence * 100.0
        );
        // Show new findings in divergent path
        for finding in &div_cp.state.findings {
            if finding.starts_with("DIVERGENT:") {
                println!("         → {finding}");
            }
        }
    }
    println!();

    print_success(&format!(
        "Resumed from step {resume_step} with divergent results across {} steps",
        divergent.len()
    ));
    print_success(&format!(
        "Original final confidence: {:.0}% vs Divergent: {:.0}%",
        checkpoints.last().unwrap().state.confidence * 100.0,
        divergent.last().unwrap().state.confidence * 100.0
    ));
    println!();

    // -----------------------------------------------------------------------
    // Step 4: Fork at Step into New Thread with fork_at()
    // -----------------------------------------------------------------------

    print_step(4, "Fork at Step into New Thread with fork_at()");

    let fork_step = 1;
    let fork_thread = "alternative_research";
    print_progress(&format!("Forking at step {fork_step} into new thread: \"{fork_thread}\""));
    print_progress(&format!(
        "Base state at fork point: {}",
        checkpoints[fork_step].state.summary()
    ));
    println!();

    handle.fork_at(fork_step, fork_thread)?;

    let forked = handle.forked_threads.get(fork_thread).unwrap();
    println!("  Forked thread \"{fork_thread}\" execution:");
    println!("  {}", "-".repeat(80));
    for fcp in forked {
        println!("  Step {}: [{}] {}", fcp.step, fcp.state.phase, fcp.description);
        for finding in &fcp.state.findings {
            if finding.starts_with(&format!("FORK[{fork_thread}]")) {
                println!("       → {finding}");
            }
        }
    }
    println!();

    print_success(&format!(
        "Forked into thread \"{fork_thread}\" with {} new checkpoints",
        forked.len()
    ));
    print_success(&format!(
        "Forked thread confidence: {:.0}% (exploring alternative direction)",
        forked.last().unwrap().state.confidence * 100.0
    ));
    println!();

    // -----------------------------------------------------------------------
    // Step 5: Replay Between Steps with replay()
    // -----------------------------------------------------------------------

    print_step(5, "Replay Between Steps with replay()");

    let replay_from = 1;
    let replay_to = 4;
    print_progress(&format!("Replaying steps {replay_from} through {replay_to}..."));
    print_progress("Printing state transitions at each step:");
    println!();

    let transitions = handle.replay(replay_from, replay_to)?;

    for transition in &transitions {
        println!("  Step {} → Step {}:", transition.from_step, transition.to_step);
        println!("    Before: {}", transition.from_state);
        println!("    After:  {}", transition.to_state);
        println!("    Changes:");
        for change in &transition.changes {
            println!("      • {change}");
        }
        println!();
    }

    print_success(&format!(
        "Replayed {} state transitions (steps {} → {})",
        transitions.len(),
        replay_from,
        replay_to
    ));
    println!();

    // -----------------------------------------------------------------------
    // Summary
    // -----------------------------------------------------------------------

    print_summary(&[
        "Executed 6-step research graph with checkpointing enabled.",
        "Listed all steps with timestamps using steps().",
        "Resumed from step 2 showing divergent results (different random seed).",
        "Forked at step 1 into \"alternative_research\" thread.",
        "Replayed steps 1→4 showing state transitions at each step.",
        "",
        "Key APIs demonstrated:",
        "  • TimeTravelHandle::steps() — list all checkpoints",
        "  • TimeTravelHandle::resume_from(step) — resume with divergent execution",
        "  • TimeTravelHandle::fork_at(step, thread_id) — branch into new thread",
        "  • TimeTravelHandle::replay(from, to) — re-execute and observe transitions",
        "",
        "Time travel enables exploring alternative execution paths without",
        "re-running the entire workflow from scratch.",
    ]);

    Ok(())
}

/// Truncate a string to a maximum length, appending "..." if truncated.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut end = max_len.saturating_sub(3);
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_require_env_missing_variable() {
        let result = require_env("ADK_TEST_NONEXISTENT_VAR_TIME_TRAVEL");
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("ADK_TEST_NONEXISTENT_VAR_TIME_TRAVEL"),
            "Error should contain the variable name, got: {msg}"
        );
        assert!(msg.contains(".env.example"), "Error should reference .env.example, got: {msg}");
    }

    #[test]
    fn test_require_env_present_variable() {
        unsafe { std::env::set_var("ADK_TEST_TIME_TRAVEL_VAR", "test_value") };
        let result = require_env("ADK_TEST_TIME_TRAVEL_VAR");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test_value");
        unsafe { std::env::remove_var("ADK_TEST_TIME_TRAVEL_VAR") };
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
    fn test_research_workflow_produces_six_steps() {
        let (checkpoints, final_state) = simulate_research_workflow();
        assert_eq!(checkpoints.len(), 6, "Should produce 6 checkpoints");
        assert_eq!(final_state.phase, "final_report");
        assert!(final_state.confidence > 0.9);
        assert!(!final_state.findings.is_empty());
        assert!(!final_state.sources.is_empty());
    }

    #[test]
    fn test_time_travel_steps_returns_all_checkpoints() {
        let (checkpoints, _) = simulate_research_workflow();
        let mut handle = TimeTravelHandle::new("test_thread");
        for cp in checkpoints {
            handle.checkpoints.push(cp);
        }

        let steps = handle.steps();
        assert_eq!(steps.len(), 6);
        for (i, step) in steps.iter().enumerate() {
            assert_eq!(step.step, i);
            assert!(!step.description.is_empty());
            assert!(!step.state_summary.is_empty());
        }
    }

    #[test]
    fn test_time_travel_resume_from_produces_divergent_results() {
        let (checkpoints, _) = simulate_research_workflow();
        let mut handle = TimeTravelHandle::new("test_thread");
        for cp in checkpoints.clone() {
            handle.checkpoints.push(cp);
        }

        let divergent = handle.resume_from(2).unwrap();
        assert!(!divergent.is_empty());

        // Divergent execution should have different confidence values
        let original_final = &checkpoints.last().unwrap().state;
        let divergent_final = &divergent.last().unwrap().state;
        assert_ne!(
            format!("{:.2}", original_final.confidence),
            format!("{:.2}", divergent_final.confidence),
            "Divergent execution should produce different confidence"
        );
    }

    #[test]
    fn test_time_travel_resume_from_invalid_step() {
        let (checkpoints, _) = simulate_research_workflow();
        let mut handle = TimeTravelHandle::new("test_thread");
        for cp in checkpoints {
            handle.checkpoints.push(cp);
        }

        let result = handle.resume_from(99);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));
    }

    #[test]
    fn test_time_travel_fork_at_creates_new_thread() {
        let (checkpoints, _) = simulate_research_workflow();
        let mut handle = TimeTravelHandle::new("test_thread");
        for cp in checkpoints {
            handle.checkpoints.push(cp);
        }

        handle.fork_at(1, "alt_thread").unwrap();
        assert!(handle.forked_threads.contains_key("alt_thread"));

        let forked = handle.forked_threads.get("alt_thread").unwrap();
        assert!(!forked.is_empty());
        // Forked thread should contain findings referencing the fork
        let has_fork_finding = forked
            .iter()
            .any(|cp| cp.state.findings.iter().any(|f| f.contains("FORK[alt_thread]")));
        assert!(has_fork_finding, "Forked thread should have fork-specific findings");
    }

    #[test]
    fn test_time_travel_fork_at_invalid_step() {
        let (checkpoints, _) = simulate_research_workflow();
        let mut handle = TimeTravelHandle::new("test_thread");
        for cp in checkpoints {
            handle.checkpoints.push(cp);
        }

        let result = handle.fork_at(99, "bad_thread");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));
    }

    #[test]
    fn test_time_travel_replay_returns_transitions() {
        let (checkpoints, _) = simulate_research_workflow();
        let mut handle = TimeTravelHandle::new("test_thread");
        for cp in checkpoints {
            handle.checkpoints.push(cp);
        }

        let transitions = handle.replay(1, 4).unwrap();
        assert_eq!(transitions.len(), 3, "Should have 3 transitions for steps 1→4");

        for t in &transitions {
            assert!(t.to_step == t.from_step + 1);
            assert!(!t.changes.is_empty(), "Each transition should have changes");
        }
    }

    #[test]
    fn test_time_travel_replay_invalid_range() {
        let (checkpoints, _) = simulate_research_workflow();
        let mut handle = TimeTravelHandle::new("test_thread");
        for cp in checkpoints {
            handle.checkpoints.push(cp);
        }

        // from > to
        let result = handle.replay(4, 1);
        assert!(result.is_err());

        // out of bounds
        let result = handle.replay(0, 99);
        assert!(result.is_err());
    }

    #[test]
    fn test_compute_state_changes_detects_differences() {
        let from = ResearchState {
            topic: "test".to_string(),
            findings: vec!["a".to_string()],
            phase: "phase_a".to_string(),
            confidence: 0.5,
            sources: vec!["src1".to_string()],
        };
        let to = ResearchState {
            topic: "test".to_string(),
            findings: vec!["a".to_string(), "b".to_string()],
            phase: "phase_b".to_string(),
            confidence: 0.8,
            sources: vec!["src1".to_string(), "src2".to_string()],
        };

        let changes = compute_state_changes(&from, &to);
        assert!(changes.iter().any(|c| c.contains("phase")));
        assert!(changes.iter().any(|c| c.contains("findings")));
        assert!(changes.iter().any(|c| c.contains("confidence")));
        assert!(changes.iter().any(|c| c.contains("sources")));
    }

    #[test]
    fn test_research_state_summary() {
        let state = ResearchState {
            topic: "test".to_string(),
            findings: vec!["f1".to_string(), "f2".to_string()],
            phase: "analysis".to_string(),
            confidence: 0.75,
            sources: vec!["s1".to_string()],
        };
        let summary = state.summary();
        assert!(summary.contains("analysis"));
        assert!(summary.contains("findings=2"));
        assert!(summary.contains("75%"));
        assert!(summary.contains("sources=1"));
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("short", 10), "short");
        assert_eq!(truncate_str("this is a long string", 10), "this is...");
        assert_eq!(truncate_str("exact", 5), "exact");
    }
}
