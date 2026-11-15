//! Functional Workflow Example
//!
//! Demonstrates the Functional API from `adk-graph`: TaskContext, ReducedValue,
//! UntrackedValue, MessagesValue, StateSchemaValidator, ExecutionLog, and
//! iteration keys for loop checkpointing.
//!
//! This example is fully self-contained — no API keys required.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use serde_json::json;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use adk_graph::checkpoint::MemoryCheckpointer;
use adk_graph::functional::{
    ChatMessage, ExecutionLog, ExpectedType, MessagesValue, MessageRole, ReducedValue,
    StateSchemaValidator, TaskContext, UntrackedValue,
};
use adk_graph::state::{State, StateSchema};
use adk_graph::stream::StreamEvent;

fn print_banner() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║        Functional Workflow API — Complete Example            ║");
    println!("║                                                              ║");
    println!("║  Demonstrates: TaskContext, ReducedValue, UntrackedValue,    ║");
    println!("║  MessagesValue, StateSchemaValidator, ExecutionLog,          ║");
    println!("║  and iteration keys for loop checkpointing.                  ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
}

fn print_section(title: &str) {
    println!("\n┌─────────────────────────────────────────────────────────────┐");
    println!("│ {title:<60}│");
    println!("└─────────────────────────────────────────────────────────────┘");
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(false)
        .init();

    print_banner();

    // ─── Step 1: Create TaskContext with MemoryCheckpointer ──────────────
    print_section("Step 1: Creating TaskContext with MemoryCheckpointer");

    let checkpointer = Arc::new(MemoryCheckpointer::new());
    let (event_tx, mut event_rx) = tokio::sync::broadcast::channel::<StreamEvent>(32);
    let execution_log = Arc::new(RwLock::new(ExecutionLog::new()));
    let cancel_token = CancellationToken::new();

    // Define a schema with channels
    let schema = StateSchema::builder()
        .channel("status")
        .counter_channel("step_count")
        .list_channel("results")
        .build();

    // Initialize state with schema defaults
    let mut initial_state = schema.initialize_state();
    initial_state.insert("status".to_string(), json!("initialized"));

    let ctx = TaskContext::new(
        "thread-example-001".to_string(),
        initial_state,
        checkpointer.clone(),
        event_tx.clone(),
        execution_log.clone(),
        cancel_token.clone(),
        Some(schema),
    );

    println!("  ✓ TaskContext created with thread_id = {:?}", ctx.thread_id());
    println!("  ✓ MemoryCheckpointer attached");
    println!("  ✓ StateSchema: channels [status, step_count, results]");
    println!("  ✓ Initial state: {:?}", ctx.state());

    // ─── Step 2: ReducedValue — accumulate results across steps ───────────
    print_section("Step 2: ReducedValue — Accumulating Results");

    let mut results: ReducedValue<String> = ReducedValue::new();

    results.push("fetch_data: retrieved 42 records".to_string());
    results.push("validate: all records pass schema check".to_string());
    results.push("transform: normalized 42 records".to_string());

    println!("  ✓ Pushed 3 results into ReducedValue");
    println!("  ✓ Length: {}", results.len());
    println!("  ✓ Contents:");
    for (i, item) in results.iter().enumerate() {
        println!("      [{i}] {item}");
    }

    // Demonstrate persistence via serialization round-trip
    let serialized = serde_json::to_string(&results)?;
    let deserialized: ReducedValue<String> = serde_json::from_str(&serialized)?;
    println!("  ✓ Round-trip serialization: {} items preserved", deserialized.len());
    assert_eq!(results.len(), deserialized.len());

    // ─── Step 3: UntrackedValue — transient data excluded from checkpoints ──
    print_section("Step 3: UntrackedValue — Transient Data");

    let mut cache: UntrackedValue<Vec<String>> = UntrackedValue::new();
    println!("  ✓ Created UntrackedValue<Vec<String>>, initial: {:?}", cache.get());

    cache.set(vec![
        "temp_computation_1".to_string(),
        "temp_computation_2".to_string(),
    ]);
    println!("  ✓ Set transient data: {:?}", cache.get());

    // Show that serialization produces null (excluded from checkpoints)
    let serialized = serde_json::to_string(&cache)?;
    println!("  ✓ Serialized form: {serialized} (always null — excluded from checkpoint)");

    // After deserialization, value resets to default
    let restored: UntrackedValue<Vec<String>> = serde_json::from_str(&serialized)?;
    println!("  ✓ After restore: {:?} (reset to default)", restored.get());
    assert!(restored.get().is_empty());

    // ─── Step 4: MessagesValue — chat messages with dedup ────────────────
    print_section("Step 4: MessagesValue — Chat Messages with Dedup");

    let mut messages = MessagesValue::new();

    messages.push(ChatMessage {
        id: "msg-001".to_string(),
        role: MessageRole::User,
        content: "Analyze the quarterly report".to_string(),
        metadata: None,
    });
    messages.push(ChatMessage {
        id: "msg-002".to_string(),
        role: MessageRole::Assistant,
        content: "I'll analyze the Q3 report now.".to_string(),
        metadata: Some(json!({"model": "gemini-2.5-flash"})),
    });
    println!("  ✓ Pushed 2 messages (user + assistant)");
    println!("  ✓ Message count: {}", messages.len());

    // Demonstrate dedup: push with same ID replaces content
    messages.push(ChatMessage {
        id: "msg-002".to_string(),
        role: MessageRole::Assistant,
        content: "Analysis complete: revenue up 12% QoQ.".to_string(),
        metadata: Some(json!({"model": "gemini-2.5-flash", "updated": true})),
    });
    println!("  ✓ Re-pushed msg-002 with updated content (dedup)");
    println!("  ✓ Message count still: {} (dedup worked!)", messages.len());
    assert_eq!(messages.len(), 2);

    // Filter by role
    let assistant_msgs = messages.by_role(MessageRole::Assistant);
    println!("  ✓ Assistant messages: {}", assistant_msgs.len());
    println!(
        "      Content: {:?}",
        assistant_msgs[0].content
    );

    // Serialization round-trip preserves dedup index
    let json_messages = serde_json::to_string(&messages)?;
    let mut restored_messages: MessagesValue = serde_json::from_str(&json_messages)?;
    restored_messages.push(ChatMessage {
        id: "msg-002".to_string(),
        role: MessageRole::Assistant,
        content: "Final answer: revenue increased.".to_string(),
        metadata: None,
    });
    println!("  ✓ After round-trip + dedup push: count = {}", restored_messages.len());
    assert_eq!(restored_messages.len(), 2);

    // ─── Step 5: StateSchemaValidator — validate state ───────────────────
    print_section("Step 5: StateSchemaValidator — Type Validation");

    let validator_schema = StateSchema::builder()
        .channel("status")
        .counter_channel("step_count")
        .list_channel("results")
        .build();

    let validator = StateSchemaValidator::new(validator_schema)
        .expect_type("status", ExpectedType::String)
        .expect_type("step_count", ExpectedType::Number)
        .expect_type("results", ExpectedType::Array)
        .require_field("status");

    // Validate the current context state
    let valid_state = ctx.state().clone();
    match validator.validate_state(&valid_state) {
        Ok(()) => println!("  ✓ Current state passes validation"),
        Err(e) => println!("  ✗ Validation failed: {e}"),
    }

    // Show validation failure: wrong type
    let mut bad_state: State = HashMap::new();
    bad_state.insert("status".to_string(), json!(42)); // Number, not String!
    bad_state.insert("step_count".to_string(), json!(0));

    match validator.validate_state(&bad_state) {
        Ok(()) => println!("  ✗ Should have failed!"),
        Err(e) => println!("  ✓ Caught type violation: {e}"),
    }

    // Show validation failure: missing required field
    let empty_state: State = HashMap::new();
    match validator.validate_state(&empty_state) {
        Ok(()) => println!("  ✗ Should have failed!"),
        Err(e) => println!("  ✓ Caught missing required field: {e}"),
    }

    // Validate task output before applying reducers
    let mut task_output: State = HashMap::new();
    task_output.insert("step_count".to_string(), json!(1));
    task_output.insert("results".to_string(), json!(["new_result"]));
    match validator.validate_task_output(&task_output) {
        Ok(()) => println!("  ✓ Task output passes validation"),
        Err(e) => println!("  ✗ Task output validation failed: {e}"),
    }

    // Attach validator to context
    let ctx = ctx.with_schema_validator(validator.clone());
    match ctx.validate_state() {
        Ok(()) => println!("  ✓ Context state validation via attached validator: PASS"),
        Err(e) => println!("  ✗ Context validation failed: {e}"),
    }

    // ─── Step 6: ExecutionLog — task completion tracking ─────────────────
    print_section("Step 6: ExecutionLog — Task Completion Tracking");

    let mut log = ExecutionLog::new();
    println!("  ✓ Created ExecutionLog, current_step: {}", log.current_step());

    // Simulate task lifecycle
    log.record_start("fetch_data");
    println!("  ✓ Recorded start: 'fetch_data'");

    log.record_completion("fetch_data", json!({"records": 42}));
    println!("  ✓ Recorded completion: 'fetch_data'");
    println!("    is_completed('fetch_data'): {}", log.is_completed("fetch_data"));
    println!("    get_result('fetch_data'): {:?}", log.get_result("fetch_data"));

    let step = log.advance_step();
    println!("  ✓ Advanced step to: {step}");

    log.record_start("validate");
    log.record_completion("validate", json!({"valid": true}));
    let step = log.advance_step();
    println!("  ✓ Completed 'validate', step: {step}");

    log.record_start("transform");
    log.record_failure("transform", "network timeout during transformation");
    println!("  ✓ Recorded failure: 'transform'");
    println!("    is_completed('transform'): {}", log.is_completed("transform"));

    // Show resume-skip behavior
    println!("\n  Resume-skip demonstration:");
    let tasks = ["fetch_data", "validate", "transform", "finalize"];
    for task in tasks {
        if log.is_completed(task) {
            let cached = log.get_result(task);
            println!("    ⏭ Skipping '{task}' (already completed, cached: {cached:?})");
        } else {
            println!("    ▶ Would execute '{task}' (not yet completed)");
        }
    }

    // Show serialization (for checkpoint persistence)
    let _log_json = serde_json::to_value(&log)?;
    println!("  ✓ ExecutionLog serialized for checkpoint");
    println!("    Tasks tracked: {}", log.tasks.len());

    // ─── Step 7: Iteration Keys — loop checkpointing ─────────────────────
    print_section("Step 7: Iteration Keys — Loop Checkpointing");

    // Reconstruct a mutable context for iteration key demo
    let (event_tx2, _rx2) = tokio::sync::broadcast::channel::<StreamEvent>(8);
    let execution_log2 = Arc::new(RwLock::new(ExecutionLog::new()));
    let mut ctx2 = TaskContext::new(
        "thread-loop-demo".to_string(),
        HashMap::new(),
        checkpointer.clone(),
        event_tx2,
        execution_log2,
        CancellationToken::new(),
        None,
    );

    let items = vec!["item_a", "item_b", "item_c", "item_d"];
    println!("  Processing {} items in a loop:", items.len());

    for item in &items {
        let key = ctx2.iteration_key("process_item");
        println!("    ✓ {item} → checkpoint key: \"{key}\"");
    }

    println!(
        "  ✓ Current iteration for 'process_item': {:?}",
        ctx2.current_iteration("process_item")
    );

    // Reset and re-iterate (e.g., for retry)
    ctx2.reset_iteration("process_item");
    println!("  ✓ Reset iteration counter for 'process_item'");
    let key_after_reset = ctx2.iteration_key("process_item");
    println!("  ✓ First key after reset: \"{key_after_reset}\"");
    assert_eq!(key_after_reset, "process_item::iter_0");

    // ─── Step 8: Event streaming ─────────────────────────────────────────
    print_section("Step 8: StreamEvent Emission");

    // Emit some events
    let _ = event_tx.send(StreamEvent::custom(
        "workflow",
        "progress",
        json!({"step": "fetch_data", "percent": 100}),
    ));
    let _ = event_tx.send(StreamEvent::node_start("validate", 1));
    let _ = event_tx.send(StreamEvent::node_end("validate", 1, 45));
    let _ = event_tx.send(StreamEvent::done(HashMap::new(), 3));

    // Drain received events
    let mut received = Vec::new();
    while let Ok(evt) = event_rx.try_recv() {
        received.push(evt);
    }
    println!("  ✓ Emitted and received {} stream events", received.len());
    for (i, evt) in received.iter().enumerate() {
        let evt_json = serde_json::to_string(evt)?;
        println!("    [{i}] {evt_json}");
    }

    // ─── Summary ─────────────────────────────────────────────────────────
    print_section("Summary");
    println!("  All Functional API features demonstrated successfully:");
    println!("    ✓ TaskContext with MemoryCheckpointer");
    println!("    ✓ ReducedValue — append-only accumulator");
    println!("    ✓ UntrackedValue — transient data (excluded from checkpoints)");
    println!("    ✓ MessagesValue — chat messages with ID-based dedup");
    println!("    ✓ StateSchemaValidator — type validation for state and output");
    println!("    ✓ ExecutionLog — task completion tracking and resume-skip");
    println!("    ✓ Iteration keys — deterministic loop checkpoint keys");
    println!("    ✓ StreamEvent emission and reception");
    println!();

    Ok(())
}
