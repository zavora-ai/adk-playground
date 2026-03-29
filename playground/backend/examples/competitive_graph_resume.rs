use adk_graph::prelude::*;

// Don't import adk_rust::prelude — it conflicts with adk_graph's State type
use std::sync::Arc;

// ── Durable Graph Resume — Checkpoint & Resume Execution ──
// Demonstrates adk-graph's durable execution model:
//
// 1. `MemoryCheckpointer` saves graph state after each node
// 2. If execution fails mid-graph, it resumes from the last checkpoint
// 3. `StreamEvent::Resumed` signals when a graph resumes from saved state
//
// This is critical for long-running agent workflows — if a node fails or
// the process restarts, work isn't lost. The graph picks up where it left off.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== Durable Graph Resume — Checkpoint & Recovery ===\n");

    let checkpointer = Arc::new(MemoryCheckpointer::default());

    // ── Part 1: Normal execution with checkpointing ──
    println!("── Part 1: Full Execution with Checkpoints ──\n");

    let graph = build_pipeline("pipeline_v1", checkpointer.clone());

    let mut input = State::new();
    input.insert("data".to_string(), json!("raw customer feedback"));
    input.insert("stage".to_string(), json!("start"));

    let config = ExecutionConfig::new("job-001");
    let result = graph.invoke(input, config).await?;

    println!("\n📊 Final state:");
    println!("   data:  {}", result.get("data").unwrap_or(&json!("?")));
    println!("   stage: {}", result.get("stage").unwrap_or(&json!("?")));
    println!("   steps: {}", result.get("steps_completed").unwrap_or(&json!(0)));

    // ── Part 2: Simulate resume from checkpoint ──
    println!("\n── Part 2: Resume from Checkpoint ──\n");
    println!("Simulating: process crashed after 'analyze' node completed.");
    println!("Pre-saving checkpoint as if 'analyze' already ran...\n");

    let mut saved_state = State::new();
    saved_state.insert("data".to_string(), json!("analyzed: sentiment=positive, topics=[quality, speed]"));
    saved_state.insert("stage".to_string(), json!("analyzed"));
    saved_state.insert("steps_completed".to_string(), json!(2));

    let checkpoint = Checkpoint::new(
        "job-002",
        saved_state,
        2,                                    // step 2 completed
        vec!["summarize".to_string()],        // next node to run
    );
    checkpointer.save(&checkpoint).await?;
    println!("💾 Checkpoint saved: thread=job-002, step=2, pending=[summarize]");

    let graph2 = build_pipeline("pipeline_v2", checkpointer.clone());
    let empty_input = State::new(); // no input needed — state comes from checkpoint
    let config2 = ExecutionConfig::new("job-002");
    let result2 = graph2.invoke(empty_input, config2).await?;

    println!("\n📊 Resumed result:");
    println!("   data:  {}", result2.get("data").unwrap_or(&json!("?")));
    println!("   stage: {}", result2.get("stage").unwrap_or(&json!("?")));
    println!("   steps: {}", result2.get("steps_completed").unwrap_or(&json!(0)));

    // ── Part 3: Verify checkpoint lifecycle ──
    println!("\n── Part 3: Checkpoint Lifecycle ──\n");

    let list = checkpointer.list("job-001").await?;
    println!("📋 Checkpoints for job-001: {} saved", list.len());

    let loaded = checkpointer.load("job-001").await?;
    if let Some(cp) = loaded {
        println!("   Latest: step={}, state keys={:?}", cp.step, cp.state.keys().collect::<Vec<_>>());
    }

    checkpointer.delete("job-001").await?;
    let after = checkpointer.load("job-001").await?;
    println!("   After delete: {}", if after.is_none() { "cleaned up ✓" } else { "still exists ✗" });

    println!("\n=== Key Features ===");
    println!("• MemoryCheckpointer — in-memory checkpoint storage (also SqliteCheckpointer available)");
    println!("• Checkpoint::new(thread, state, step, pending) — save execution progress");
    println!("• Graph auto-resumes from last checkpoint on invoke()");
    println!("• Skips already-completed nodes — no duplicate work");
    println!("• Critical for long-running multi-agent pipelines");
    Ok(())
}

fn build_pipeline(name: &str, checkpointer: Arc<MemoryCheckpointer>) -> GraphAgent {
    GraphAgent::builder(name)
        .description("3-stage data pipeline with checkpointing")
        .state_schema(
            StateSchemaBuilder::default()
                .channel_with_default("data", json!(""))
                .channel_with_default("stage", json!("start"))
                .channel_with_default("steps_completed", json!(0))
                .build(),
        )
        .node_fn("ingest", |ctx: NodeContext| async move {
            let data = ctx.state.get("data").and_then(|v| v.as_str()).unwrap_or("");
            println!("  📥 Ingest: received {} chars", data.len());
            let steps = ctx.state.get("steps_completed").and_then(|v| v.as_i64()).unwrap_or(0);
            Ok(NodeOutput::new()
                .with_update("data", json!(format!("ingested: {}", data)))
                .with_update("stage", json!("ingested"))
                .with_update("steps_completed", json!(steps + 1)))
        })
        .node_fn("analyze", |ctx: NodeContext| async move {
            let data = ctx.state.get("data").and_then(|v| v.as_str()).unwrap_or("");
            println!("  🔍 Analyze: processing {} chars", data.len());
            let steps = ctx.state.get("steps_completed").and_then(|v| v.as_i64()).unwrap_or(0);
            Ok(NodeOutput::new()
                .with_update("data", json!("analyzed: sentiment=positive, topics=[quality, speed]"))
                .with_update("stage", json!("analyzed"))
                .with_update("steps_completed", json!(steps + 1)))
        })
        .node_fn("summarize", |ctx: NodeContext| async move {
            let data = ctx.state.get("data").and_then(|v| v.as_str()).unwrap_or("");
            println!("  📝 Summarize: condensing analysis");
            let steps = ctx.state.get("steps_completed").and_then(|v| v.as_i64()).unwrap_or(0);
            Ok(NodeOutput::new()
                .with_update("data", json!(format!("summary of [{}] → positive feedback on quality and speed", data)))
                .with_update("stage", json!("complete"))
                .with_update("steps_completed", json!(steps + 1)))
        })
        .edge(START, "ingest")
        .edge("ingest", "analyze")
        .edge("analyze", "summarize")
        .edge("summarize", END)
        .checkpointer_arc(checkpointer)
        .build()
        .expect("graph should build")
}
