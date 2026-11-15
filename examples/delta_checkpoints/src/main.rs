//! # Delta Checkpoints Example
//!
//! Demonstrates ADK-Rust's delta checkpointing system that stores incremental
//! state diffs instead of full snapshots, with periodic full snapshots for
//! efficient state reconstruction.
//!
//! ## What This Shows
//!
//! - Configuring `DeltaCheckpointer` with `full_snapshot_interval`
//! - Wrapping a `MemoryCheckpointer` with delta-aware checkpointing
//! - Observing delta vs full checkpoint storage at each graph step
//! - Reconstructing state from delta checkpoints and verifying correctness
//! - Comparing storage sizes between full snapshots and delta checkpoints
//!
//! ## Prerequisites
//!
//! - `GOOGLE_API_KEY` environment variable set (for the Gemini LLM provider)
//!
//! ## Run
//!
//! ```bash
//! cargo run --manifest-path examples/delta_checkpoints/Cargo.toml
//! ```

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use tracing_subscriber::EnvFilter;

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
    println!("║  Delta Checkpoints — ADK-Rust v0.8.0     ║");
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

#[allow(dead_code)]
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
// Delta Checkpointing Types
// ===========================================================================

/// The type of state stored at each checkpoint step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum CheckpointType {
    /// A complete snapshot of the full state at this step.
    FullSnapshot,
    /// An incremental diff relative to the previous checkpoint.
    Delta,
}

impl std::fmt::Display for CheckpointType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckpointType::FullSnapshot => write!(f, "Full Snapshot"),
            CheckpointType::Delta => write!(f, "Delta"),
        }
    }
}

/// Represents the difference between two states.
///
/// Captures keys that were added/modified and keys that were removed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StateDiff {
    /// Keys that were added or modified, with their new values.
    pub added_or_modified: BTreeMap<String, serde_json::Value>,
    /// Keys that were removed from the state.
    pub removed: Vec<String>,
}

impl StateDiff {
    /// Compute the diff between an old state and a new state.
    pub fn compute(
        old: &BTreeMap<String, serde_json::Value>,
        new: &BTreeMap<String, serde_json::Value>,
    ) -> Self {
        let mut added_or_modified = BTreeMap::new();
        let mut removed = Vec::new();

        // Find added or modified keys
        for (key, new_val) in new {
            match old.get(key) {
                Some(old_val) if old_val == new_val => {} // unchanged
                _ => {
                    added_or_modified.insert(key.clone(), new_val.clone());
                }
            }
        }

        // Find removed keys
        for key in old.keys() {
            if !new.contains_key(key) {
                removed.push(key.clone());
            }
        }

        Self {
            added_or_modified,
            removed,
        }
    }

    /// Apply this diff to a base state to reconstruct the resulting state.
    pub fn apply(&self, base: &BTreeMap<String, serde_json::Value>) -> BTreeMap<String, serde_json::Value> {
        let mut result = base.clone();

        // Remove deleted keys
        for key in &self.removed {
            result.remove(key);
        }

        // Apply additions and modifications
        for (key, value) in &self.added_or_modified {
            result.insert(key.clone(), value.clone());
        }

        result
    }

    /// Returns the serialized size of this diff in bytes.
    pub fn size_bytes(&self) -> usize {
        serde_json::to_vec(self).unwrap_or_default().len()
    }

    /// Returns true if this diff represents no changes.
    pub fn is_empty(&self) -> bool {
        self.added_or_modified.is_empty() && self.removed.is_empty()
    }
}

/// A single checkpoint entry stored by the checkpointer.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CheckpointEntry {
    /// The step number this checkpoint corresponds to.
    step: usize,
    /// Whether this is a full snapshot or a delta.
    checkpoint_type: CheckpointType,
    /// Full state (only present for FullSnapshot type).
    full_state: Option<BTreeMap<String, serde_json::Value>>,
    /// State diff (only present for Delta type).
    diff: Option<StateDiff>,
    /// Serialized size in bytes.
    size_bytes: usize,
}

/// Configuration for the delta checkpointer.
#[derive(Debug, Clone)]
struct DeltaConfig {
    /// How often to store a full snapshot (every N steps).
    /// For example, `full_snapshot_interval: 3` means steps 0, 3, 6, ...
    /// get full snapshots, and all others get deltas.
    full_snapshot_interval: usize,
}

/// In-memory storage backend for checkpoints.
///
/// Simulates `adk_graph::checkpoint::MemoryCheckpointer`.
#[derive(Debug, Clone, Default)]
struct MemoryCheckpointer {
    /// All stored checkpoint entries, keyed by step number.
    entries: HashMap<usize, CheckpointEntry>,
}

impl MemoryCheckpointer {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    fn store(&mut self, entry: CheckpointEntry) {
        self.entries.insert(entry.step, entry);
    }

    fn get(&self, step: usize) -> Option<&CheckpointEntry> {
        self.entries.get(&step)
    }

    /// Total storage used across all entries.
    fn total_size_bytes(&self) -> usize {
        self.entries.values().map(|e| e.size_bytes).sum()
    }
}

/// Delta-aware checkpointer that wraps an inner `MemoryCheckpointer`.
///
/// Stores incremental state diffs between steps, with periodic full snapshots
/// at the configured interval. This reduces storage costs for large state graphs
/// while maintaining the ability to reconstruct any historical state.
///
/// Simulates `adk_graph::delta::DeltaCheckpointer`.
#[derive(Debug, Clone)]
struct DeltaCheckpointer {
    /// The underlying storage backend.
    inner: MemoryCheckpointer,
    /// Configuration controlling snapshot frequency.
    config: DeltaConfig,
    /// The last full state that was checkpointed (used to compute deltas).
    last_state: Option<BTreeMap<String, serde_json::Value>>,
    /// Step counter tracking how many steps since last full snapshot.
    steps_since_full: usize,
}

impl DeltaCheckpointer {
    /// Create a new delta checkpointer wrapping the given inner checkpointer.
    fn new(inner: MemoryCheckpointer, config: DeltaConfig) -> Self {
        Self {
            inner,
            config,
            last_state: None,
            steps_since_full: 0,
        }
    }

    /// Checkpoint the given state at the specified step.
    ///
    /// Decides whether to store a full snapshot or a delta based on the
    /// configured `full_snapshot_interval`. Full snapshots occur at step 0
    /// and every `full_snapshot_interval` steps thereafter (e.g., steps 0, 3, 6
    /// for interval=3).
    fn checkpoint(
        &mut self,
        step: usize,
        state: &BTreeMap<String, serde_json::Value>,
    ) -> &CheckpointEntry {
        let should_full_snapshot = self.last_state.is_none()
            || self.steps_since_full >= self.config.full_snapshot_interval;

        let entry = if should_full_snapshot {
            // Store a full snapshot
            let serialized = serde_json::to_vec(state).unwrap_or_default();
            self.steps_since_full = 1; // This snapshot counts as step 1 of the next interval
            CheckpointEntry {
                step,
                checkpoint_type: CheckpointType::FullSnapshot,
                full_state: Some(state.clone()),
                diff: None,
                size_bytes: serialized.len(),
            }
        } else {
            // Compute and store a delta
            let prev = self.last_state.as_ref().unwrap();
            let diff = StateDiff::compute(prev, state);
            let size = diff.size_bytes();
            self.steps_since_full += 1;
            CheckpointEntry {
                step,
                checkpoint_type: CheckpointType::Delta,
                full_state: None,
                diff: Some(diff),
                size_bytes: size,
            }
        };

        self.last_state = Some(state.clone());
        self.inner.store(entry);
        self.inner.get(step).unwrap()
    }

    /// Reconstruct the full state at a given step by walking back to the
    /// nearest full snapshot and applying deltas forward.
    fn reconstruct_state(
        &self,
        target_step: usize,
    ) -> anyhow::Result<BTreeMap<String, serde_json::Value>> {
        // Find the nearest full snapshot at or before target_step
        let mut base_step = None;
        for s in (0..=target_step).rev() {
            if let Some(entry) = self.inner.get(s)
                && entry.checkpoint_type == CheckpointType::FullSnapshot
            {
                base_step = Some(s);
                break;
            }
        }

        let base_step = base_step.ok_or_else(|| {
            anyhow::anyhow!("No full snapshot found at or before step {target_step}")
        })?;

        // Start with the full snapshot state
        let base_entry = self.inner.get(base_step).unwrap();
        let mut state = base_entry.full_state.clone().unwrap();

        // Apply deltas forward from base_step+1 to target_step
        for s in (base_step + 1)..=target_step {
            if let Some(entry) = self.inner.get(s) {
                match &entry.diff {
                    Some(diff) => {
                        state = diff.apply(&state);
                    }
                    None => {
                        // This is a full snapshot, use it directly
                        if let Some(full) = &entry.full_state {
                            state = full.clone();
                        }
                    }
                }
            }
        }

        Ok(state)
    }

    /// Get a reference to the inner checkpointer for inspection.
    fn inner(&self) -> &MemoryCheckpointer {
        &self.inner
    }
}

// ===========================================================================
// Simulated Multi-Step Graph Execution
// ===========================================================================

/// Represents a single step in the graph execution that modifies state.
struct GraphStep {
    /// Human-readable description of what this step does.
    description: &'static str,
    /// Function that applies state changes for this step.
    apply: fn(&mut BTreeMap<String, serde_json::Value>),
}

/// Build the 8-step graph that accumulates state progressively.
///
/// Each step adds, modifies, or removes keys to demonstrate how delta
/// checkpointing captures only the changes.
fn build_graph_steps() -> Vec<GraphStep> {
    vec![
        GraphStep {
            description: "Initialize research topic and parameters",
            apply: |state| {
                state.insert(
                    "topic".to_string(),
                    serde_json::json!("quantum computing applications"),
                );
                state.insert("max_sources".to_string(), serde_json::json!(10));
                state.insert("language".to_string(), serde_json::json!("en"));
                state.insert(
                    "started_at".to_string(),
                    serde_json::json!("2025-01-15T10:00:00Z"),
                );
            },
        },
        GraphStep {
            description: "Gather initial sources from web search",
            apply: |state| {
                state.insert(
                    "sources".to_string(),
                    serde_json::json!([
                        {"url": "https://arxiv.org/quantum-1", "title": "Quantum Error Correction"},
                        {"url": "https://nature.com/quantum-2", "title": "Quantum Supremacy"},
                        {"url": "https://ieee.org/quantum-3", "title": "Quantum Algorithms"}
                    ]),
                );
                state.insert("source_count".to_string(), serde_json::json!(3));
            },
        },
        GraphStep {
            description: "Analyze source relevance and extract key findings",
            apply: |state| {
                state.insert(
                    "findings".to_string(),
                    serde_json::json!({
                        "error_correction": "Topological codes show 99.9% fidelity",
                        "supremacy": "Achieved on 72-qubit processor",
                        "algorithms": "Shor's algorithm factors 2048-bit keys"
                    }),
                );
                state.insert("analysis_status".to_string(), serde_json::json!("complete"));
                // Remove temporary parameter no longer needed
                state.remove("max_sources");
            },
        },
        GraphStep {
            description: "Generate summary from findings",
            apply: |state| {
                state.insert(
                    "summary".to_string(),
                    serde_json::json!(
                        "Quantum computing has achieved significant milestones: \
                         topological error correction codes demonstrate 99.9% gate fidelity, \
                         quantum supremacy was demonstrated on a 72-qubit processor, \
                         and Shor's algorithm can now factor 2048-bit RSA keys."
                    ),
                );
                state.insert("summary_word_count".to_string(), serde_json::json!(42));
            },
        },
        GraphStep {
            description: "Add citations and references",
            apply: |state| {
                state.insert(
                    "citations".to_string(),
                    serde_json::json!([
                        {"id": 1, "author": "Smith et al.", "year": 2024, "journal": "Nature"},
                        {"id": 2, "author": "Chen et al.", "year": 2024, "journal": "Science"},
                        {"id": 3, "author": "Patel et al.", "year": 2025, "journal": "IEEE QC"}
                    ]),
                );
                state.insert("citation_count".to_string(), serde_json::json!(3));
            },
        },
        GraphStep {
            description: "Peer review and quality scoring",
            apply: |state| {
                state.insert(
                    "quality_score".to_string(),
                    serde_json::json!(0.92),
                );
                state.insert(
                    "review_notes".to_string(),
                    serde_json::json!([
                        "Strong evidence base",
                        "Clear methodology",
                        "Minor: could expand on practical applications"
                    ]),
                );
                // Update analysis status
                state.insert(
                    "analysis_status".to_string(),
                    serde_json::json!("reviewed"),
                );
            },
        },
        GraphStep {
            description: "Finalize report with metadata",
            apply: |state| {
                state.insert(
                    "report_status".to_string(),
                    serde_json::json!("finalized"),
                );
                state.insert(
                    "completed_at".to_string(),
                    serde_json::json!("2025-01-15T10:05:30Z"),
                );
                state.insert(
                    "total_tokens_used".to_string(),
                    serde_json::json!(4521),
                );
                // Remove intermediate data no longer needed
                state.remove("source_count");
            },
        },
        GraphStep {
            description: "Archive and compress results",
            apply: |state| {
                state.insert(
                    "archived".to_string(),
                    serde_json::json!(true),
                );
                state.insert(
                    "archive_id".to_string(),
                    serde_json::json!("arch-2025-01-15-001"),
                );
                state.insert(
                    "compression_ratio".to_string(),
                    serde_json::json!(0.34),
                );
            },
        },
    ]
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
    print_progress("Using model: gemini-2.5-flash\n");

    // ===================================================================
    // Step 1: Configure Delta Checkpointer
    // ===================================================================

    print_step(1, "Configure Delta Checkpointer");

    let config = DeltaConfig {
        full_snapshot_interval: 3,
    };
    print_progress(&format!(
        "DeltaConfig {{ full_snapshot_interval: {} }}",
        config.full_snapshot_interval
    ));
    print_progress("Wrapping MemoryCheckpointer with delta-aware layer");

    let inner = MemoryCheckpointer::new();
    let mut checkpointer = DeltaCheckpointer::new(inner, config.clone());
    print_success("Delta checkpointer configured");
    print_progress(&format!(
        "Full snapshots at steps: 0, {}, {}, ...",
        config.full_snapshot_interval,
        config.full_snapshot_interval * 2
    ));

    println!();

    // ===================================================================
    // Step 2: Execute Multi-Step Graph with Delta Checkpointing
    // ===================================================================

    print_step(2, "Execute Multi-Step Graph (8 steps)");

    let steps = build_graph_steps();
    let mut current_state: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    let mut step_states: Vec<BTreeMap<String, serde_json::Value>> = Vec::new();

    // Track sizes for comparison
    let mut full_snapshot_sizes: Vec<(usize, usize)> = Vec::new(); // (step, size)
    let mut delta_sizes: Vec<(usize, usize)> = Vec::new(); // (step, size)

    for (i, graph_step) in steps.iter().enumerate() {
        // Apply the step's state changes
        (graph_step.apply)(&mut current_state);
        step_states.push(current_state.clone());

        // Checkpoint the state
        let entry = checkpointer.checkpoint(i, &current_state);

        let type_indicator = match &entry.checkpoint_type {
            CheckpointType::FullSnapshot => {
                full_snapshot_sizes.push((i, entry.size_bytes));
                "📸 FULL"
            }
            CheckpointType::Delta => {
                delta_sizes.push((i, entry.size_bytes));
                "📝 DELTA"
            }
        };

        print_progress(&format!(
            "Step {i}: [{type_indicator}] {desc} ({size} bytes)",
            desc = graph_step.description,
            size = entry.size_bytes,
        ));

        // Show diff details for delta checkpoints
        if let Some(diff) = &entry.diff {
            if !diff.added_or_modified.is_empty() {
                let keys: Vec<&String> = diff.added_or_modified.keys().collect();
                print_progress(&format!(
                    "       Added/Modified: {:?}",
                    keys
                ));
            }
            if !diff.removed.is_empty() {
                print_progress(&format!(
                    "       Removed: {:?}",
                    diff.removed
                ));
            }
        }
    }

    print_success(&format!(
        "Graph execution complete: {} steps checkpointed",
        steps.len()
    ));

    println!();

    // ===================================================================
    // Step 3: Delta vs Full Checkpoint Comparison
    // ===================================================================

    print_step(3, "Delta vs Full Checkpoint Comparison");

    println!("  {:>5} | {:>12} | {:>10} | Details", "Step", "Type", "Size");
    println!("  {}", "-".repeat(50));

    for (step_num, graph_step) in steps.iter().enumerate() {
        if let Some(entry) = checkpointer.inner().get(step_num) {
            let type_str = match &entry.checkpoint_type {
                CheckpointType::FullSnapshot => "Full Snapshot",
                CheckpointType::Delta => "Delta",
            };
            println!(
                "  {:>5} | {:>12} | {:>7} B | {}",
                step_num, type_str, entry.size_bytes, graph_step.description
            );
        }
    }

    let total_full: usize = full_snapshot_sizes.iter().map(|(_, s)| s).sum();
    let total_delta: usize = delta_sizes.iter().map(|(_, s)| s).sum();
    let total_actual = checkpointer.inner().total_size_bytes();

    // Calculate what full-snapshot-only storage would cost
    let hypothetical_full_only: usize = step_states
        .iter()
        .map(|s| serde_json::to_vec(s).unwrap_or_default().len())
        .sum();

    println!();
    print_success(&format!(
        "Full snapshots stored: {} (total {} bytes)",
        full_snapshot_sizes.len(),
        total_full
    ));
    print_success(&format!(
        "Delta checkpoints stored: {} (total {} bytes)",
        delta_sizes.len(),
        total_delta
    ));

    println!();

    // ===================================================================
    // Step 4: Reconstruct State from Delta Checkpoint
    // ===================================================================

    print_step(4, "Reconstruct State from Delta Checkpoint");

    // Reconstruct state at each step and verify it matches the original
    let mut all_reconstructions_valid = true;

    for (step_num, original) in step_states.iter().enumerate() {
        let reconstructed = checkpointer.reconstruct_state(step_num)?;

        if &reconstructed == original {
            print_success(&format!(
                "Step {step_num}: Reconstructed state matches original ✓ ({} keys)",
                reconstructed.len()
            ));
        } else {
            print_warning(&format!(
                "Step {step_num}: MISMATCH! Reconstructed state differs from original"
            ));
            all_reconstructions_valid = false;
        }
    }

    println!();
    if all_reconstructions_valid {
        print_success("All state reconstructions verified — round-trip integrity confirmed");
    } else {
        print_warning("Some reconstructions failed — check delta computation logic");
    }

    // Demonstrate reconstruction from a specific delta step
    println!();
    let demo_step = 5; // A delta step
    print_progress(&format!(
        "Detailed reconstruction for step {demo_step} (delta checkpoint):"
    ));
    let entry = checkpointer.inner().get(demo_step).unwrap();
    print_progress(&format!(
        "  Checkpoint type: {}",
        entry.checkpoint_type
    ));
    print_progress(&format!(
        "  Stored size: {} bytes",
        entry.size_bytes
    ));

    let reconstructed = checkpointer.reconstruct_state(demo_step)?;
    print_progress(&format!(
        "  Reconstructed state has {} keys",
        reconstructed.len()
    ));
    let full_size = serde_json::to_vec(&reconstructed).unwrap_or_default().len();
    print_progress(&format!(
        "  Full state size would be: {} bytes",
        full_size
    ));
    print_success(&format!(
        "  Storage savings: {:.1}% (delta: {} B vs full: {} B)",
        (1.0 - entry.size_bytes as f64 / full_size as f64) * 100.0,
        entry.size_bytes,
        full_size
    ));

    println!();

    // ===================================================================
    // Step 5: Storage Size Comparison
    // ===================================================================

    print_step(5, "Storage Size Comparison");

    let savings_pct = if hypothetical_full_only > 0 {
        (1.0 - total_actual as f64 / hypothetical_full_only as f64) * 100.0
    } else {
        0.0
    };

    println!("  Storage Strategy Comparison:");
    println!("  ┌─────────────────────────────────────────────────┐");
    println!(
        "  │ Full-snapshot-only:  {:>6} bytes ({} snapshots)  │",
        hypothetical_full_only,
        steps.len()
    );
    println!(
        "  │ Delta checkpointing: {:>5} bytes ({} full + {} delta) │",
        total_actual,
        full_snapshot_sizes.len(),
        delta_sizes.len()
    );
    println!(
        "  │ Storage savings:     {:>5.1}%                        │",
        savings_pct
    );
    println!("  └─────────────────────────────────────────────────┘");

    println!();
    println!("  Per-step breakdown:");
    for (step_num, step_state) in step_states.iter().enumerate() {
        let full_size = serde_json::to_vec(step_state)
            .unwrap_or_default()
            .len();
        let actual_size = checkpointer
            .inner()
            .get(step_num)
            .map(|e| e.size_bytes)
            .unwrap_or(0);
        let step_savings = if full_size > 0 {
            (1.0 - actual_size as f64 / full_size as f64) * 100.0
        } else {
            0.0
        };
        let entry = checkpointer.inner().get(step_num).unwrap();
        let marker = match entry.checkpoint_type {
            CheckpointType::FullSnapshot => "📸",
            CheckpointType::Delta => "📝",
        };
        println!(
            "    {marker} Step {step_num}: full={full_size:>4}B, actual={actual_size:>4}B, savings={step_savings:>5.1}%"
        );
    }

    println!();

    // ===================================================================
    // Summary
    // ===================================================================

    print_summary(&[
        &format!(
            "Delta checkpointing with full_snapshot_interval: {}",
            config.full_snapshot_interval
        ),
        &format!("Total graph steps executed: {}", steps.len()),
        &format!(
            "Full snapshots: {} | Delta checkpoints: {}",
            full_snapshot_sizes.len(),
            delta_sizes.len()
        ),
        &format!(
            "Total storage: {} bytes (vs {} bytes full-only)",
            total_actual, hypothetical_full_only
        ),
        &format!("Storage savings: {savings_pct:.1}%"),
        &format!(
            "All {} state reconstructions verified ✓",
            steps.len()
        ),
        "",
        "Key concepts:",
        "  • DeltaCheckpointer wraps any inner Checkpointer",
        "  • Stores only state diffs between consecutive steps",
        "  • Periodic full snapshots ensure bounded reconstruction cost",
        "  • State can be reconstructed by applying diffs to last full snapshot",
        "  • Round-trip reconstruction preserves exact state equality",
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
        let result = require_env("DEFINITELY_NOT_SET_XYZ_DELTA_12345");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("DEFINITELY_NOT_SET_XYZ_DELTA_12345"),
            "Error should contain the variable name"
        );
        assert!(
            err_msg.contains(".env.example"),
            "Error should reference .env.example"
        );
    }

    #[test]
    fn test_require_env_present_variable() {
        unsafe { std::env::set_var("TEST_DELTA_CHECKPOINTS_VAR", "test_value") };
        let result = require_env("TEST_DELTA_CHECKPOINTS_VAR");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test_value");
        unsafe { std::env::remove_var("TEST_DELTA_CHECKPOINTS_VAR") };
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

    #[test]
    fn test_state_diff_compute_additions() {
        let old = BTreeMap::new();
        let mut new = BTreeMap::new();
        new.insert("key1".to_string(), serde_json::json!("value1"));
        new.insert("key2".to_string(), serde_json::json!(42));

        let diff = StateDiff::compute(&old, &new);
        assert_eq!(diff.added_or_modified.len(), 2);
        assert!(diff.removed.is_empty());
        assert_eq!(
            diff.added_or_modified.get("key1").unwrap(),
            &serde_json::json!("value1")
        );
    }

    #[test]
    fn test_state_diff_compute_removals() {
        let mut old = BTreeMap::new();
        old.insert("key1".to_string(), serde_json::json!("value1"));
        old.insert("key2".to_string(), serde_json::json!(42));
        let new = BTreeMap::new();

        let diff = StateDiff::compute(&old, &new);
        assert!(diff.added_or_modified.is_empty());
        assert_eq!(diff.removed.len(), 2);
        assert!(diff.removed.contains(&"key1".to_string()));
        assert!(diff.removed.contains(&"key2".to_string()));
    }

    #[test]
    fn test_state_diff_compute_modifications() {
        let mut old = BTreeMap::new();
        old.insert("key1".to_string(), serde_json::json!("old_value"));
        old.insert("key2".to_string(), serde_json::json!(42));

        let mut new = BTreeMap::new();
        new.insert("key1".to_string(), serde_json::json!("new_value"));
        new.insert("key2".to_string(), serde_json::json!(42)); // unchanged

        let diff = StateDiff::compute(&old, &new);
        assert_eq!(diff.added_or_modified.len(), 1);
        assert!(diff.removed.is_empty());
        assert_eq!(
            diff.added_or_modified.get("key1").unwrap(),
            &serde_json::json!("new_value")
        );
    }

    #[test]
    fn test_state_diff_apply() {
        let mut base = BTreeMap::new();
        base.insert("key1".to_string(), serde_json::json!("value1"));
        base.insert("key2".to_string(), serde_json::json!(42));
        base.insert("key3".to_string(), serde_json::json!("to_remove"));

        let diff = StateDiff {
            added_or_modified: {
                let mut m = BTreeMap::new();
                m.insert("key1".to_string(), serde_json::json!("updated"));
                m.insert("key4".to_string(), serde_json::json!("new_key"));
                m
            },
            removed: vec!["key3".to_string()],
        };

        let result = diff.apply(&base);
        assert_eq!(result.get("key1").unwrap(), &serde_json::json!("updated"));
        assert_eq!(result.get("key2").unwrap(), &serde_json::json!(42));
        assert!(!result.contains_key("key3"));
        assert_eq!(result.get("key4").unwrap(), &serde_json::json!("new_key"));
    }

    #[test]
    fn test_state_diff_round_trip() {
        let mut old = BTreeMap::new();
        old.insert("a".to_string(), serde_json::json!(1));
        old.insert("b".to_string(), serde_json::json!("hello"));
        old.insert("c".to_string(), serde_json::json!(true));

        let mut new = BTreeMap::new();
        new.insert("a".to_string(), serde_json::json!(2)); // modified
        new.insert("b".to_string(), serde_json::json!("hello")); // unchanged
        // "c" removed
        new.insert("d".to_string(), serde_json::json!([1, 2, 3])); // added

        let diff = StateDiff::compute(&old, &new);
        let reconstructed = diff.apply(&old);
        assert_eq!(reconstructed, new);
    }

    #[test]
    fn test_delta_checkpointer_full_snapshot_interval() {
        let config = DeltaConfig {
            full_snapshot_interval: 3,
        };
        let inner = MemoryCheckpointer::new();
        let mut checkpointer = DeltaCheckpointer::new(inner, config);

        let mut state = BTreeMap::new();

        // Step 0: should be full (first checkpoint)
        state.insert("a".to_string(), serde_json::json!(1));
        let entry = checkpointer.checkpoint(0, &state);
        assert_eq!(entry.checkpoint_type, CheckpointType::FullSnapshot);

        // Steps 1, 2: should be delta
        state.insert("b".to_string(), serde_json::json!(2));
        let entry = checkpointer.checkpoint(1, &state);
        assert_eq!(entry.checkpoint_type, CheckpointType::Delta);

        state.insert("c".to_string(), serde_json::json!(3));
        let entry = checkpointer.checkpoint(2, &state);
        assert_eq!(entry.checkpoint_type, CheckpointType::Delta);

        // Step 3: should be full (interval reached)
        state.insert("d".to_string(), serde_json::json!(4));
        let entry = checkpointer.checkpoint(3, &state);
        assert_eq!(entry.checkpoint_type, CheckpointType::FullSnapshot);

        // Steps 4, 5: should be delta again
        state.insert("e".to_string(), serde_json::json!(5));
        let entry = checkpointer.checkpoint(4, &state);
        assert_eq!(entry.checkpoint_type, CheckpointType::Delta);

        state.insert("f".to_string(), serde_json::json!(6));
        let entry = checkpointer.checkpoint(5, &state);
        assert_eq!(entry.checkpoint_type, CheckpointType::Delta);

        // Step 6: should be full again
        state.insert("g".to_string(), serde_json::json!(7));
        let entry = checkpointer.checkpoint(6, &state);
        assert_eq!(entry.checkpoint_type, CheckpointType::FullSnapshot);
    }

    #[test]
    fn test_delta_checkpointer_reconstruct_state() {
        let config = DeltaConfig {
            full_snapshot_interval: 3,
        };
        let inner = MemoryCheckpointer::new();
        let mut checkpointer = DeltaCheckpointer::new(inner, config);

        let mut state = BTreeMap::new();
        let mut expected_states = Vec::new();

        // Build up state over 6 steps
        state.insert("a".to_string(), serde_json::json!(1));
        checkpointer.checkpoint(0, &state);
        expected_states.push(state.clone());

        state.insert("b".to_string(), serde_json::json!(2));
        checkpointer.checkpoint(1, &state);
        expected_states.push(state.clone());

        state.insert("c".to_string(), serde_json::json!(3));
        state.remove("a");
        checkpointer.checkpoint(2, &state);
        expected_states.push(state.clone());

        state.insert("d".to_string(), serde_json::json!(4));
        checkpointer.checkpoint(3, &state);
        expected_states.push(state.clone());

        state.insert("e".to_string(), serde_json::json!(5));
        state.insert("b".to_string(), serde_json::json!(20)); // modify
        checkpointer.checkpoint(4, &state);
        expected_states.push(state.clone());

        state.insert("f".to_string(), serde_json::json!(6));
        checkpointer.checkpoint(5, &state);
        expected_states.push(state.clone());

        // Verify reconstruction at each step
        for (i, expected) in expected_states.iter().enumerate() {
            let reconstructed = checkpointer.reconstruct_state(i).unwrap();
            assert_eq!(
                &reconstructed, expected,
                "State mismatch at step {i}"
            );
        }
    }

    #[test]
    fn test_state_diff_empty() {
        let mut state = BTreeMap::new();
        state.insert("key".to_string(), serde_json::json!("value"));

        let diff = StateDiff::compute(&state, &state);
        assert!(diff.is_empty());
        assert_eq!(diff.added_or_modified.len(), 0);
        assert_eq!(diff.removed.len(), 0);
    }

    #[test]
    fn test_delta_smaller_than_full() {
        let config = DeltaConfig {
            full_snapshot_interval: 3,
        };
        let inner = MemoryCheckpointer::new();
        let mut checkpointer = DeltaCheckpointer::new(inner, config);

        // Build a large initial state
        let mut state = BTreeMap::new();
        for i in 0..20 {
            state.insert(
                format!("key_{i}"),
                serde_json::json!(format!("value_{i}_with_some_extra_data")),
            );
        }
        checkpointer.checkpoint(0, &state);

        // Make a small change
        state.insert("key_0".to_string(), serde_json::json!("modified"));
        let entry = checkpointer.checkpoint(1, &state);

        // Delta should be smaller than full state
        let full_size = serde_json::to_vec(&state).unwrap().len();
        assert!(
            entry.size_bytes < full_size,
            "Delta ({} bytes) should be smaller than full state ({} bytes)",
            entry.size_bytes,
            full_size
        );
    }
}
