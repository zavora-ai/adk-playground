//! PluginBuilder fluent API — validates the builder pattern for plugins
//!
//! Demonstrates: PluginBuilder chaining, selective callback registration,
//! and model-level interception hooks.

use adk_core::callbacks::BeforeModelResult;
use adk_plugin::PluginBuilder;

fn main() {
    println!("=== PluginBuilder Fluent API ===\n");

    // 1. Minimal plugin via builder
    let plugin = PluginBuilder::new("minimal").build();
    assert_eq!(plugin.name(), "minimal");
    assert!(plugin.on_user_message().is_none());
    assert!(plugin.before_model().is_none());
    println!("✓ Minimal plugin via builder (no callbacks)");

    // 2. Message-only plugin
    let plugin = PluginBuilder::new("message-logger")
        .on_user_message(Box::new(|_ctx, content| {
            Box::pin(async move {
                println!("  User: {} parts", content.parts.len());
                Ok(None)
            })
        }))
        .build();
    assert!(plugin.on_user_message().is_some());
    assert!(plugin.on_event().is_none());
    println!("✓ Message-logger plugin via builder");

    // 3. Model interceptor (before + after + error)
    let plugin = PluginBuilder::new("model-cache")
        .before_model(Box::new(|_ctx, request| {
            Box::pin(async move {
                println!("  [cache] Checking cache for request...");
                Ok(BeforeModelResult::Continue(request))
            })
        }))
        .after_model(Box::new(|_ctx, _response| {
            Box::pin(async move {
                println!("  [cache] Storing response in cache");
                Ok(None)
            })
        }))
        .on_model_error(Box::new(|_ctx, _request, error| {
            Box::pin(async move {
                println!("  [cache] Model error: {}", error);
                Ok(None)
            })
        }))
        .build();
    assert!(plugin.before_model().is_some());
    assert!(plugin.after_model().is_some());
    assert!(plugin.on_model_error().is_some());
    println!("✓ Model-cache plugin with before/after/error hooks");

    // 4. Full lifecycle plugin — all 10 hooks
    let plugin = PluginBuilder::new("full-lifecycle")
        .on_user_message(Box::new(|_ctx, _c| Box::pin(async move { Ok(None) })))
        .on_event(Box::new(|_ctx, _e| Box::pin(async move { Ok(None) })))
        .before_run(Box::new(|_ctx| Box::pin(async move { Ok(None) })))
        .after_run(Box::new(|_ctx| Box::pin(async move { () })))
        .before_agent(Box::new(|_ctx| Box::pin(async move { Ok(None) })))
        .after_agent(Box::new(|_ctx| Box::pin(async move { Ok(None) })))
        .before_model(Box::new(|_ctx, r| {
            Box::pin(async move { Ok(BeforeModelResult::Continue(r)) })
        }))
        .after_model(Box::new(|_ctx, _r| Box::pin(async move { Ok(None) })))
        .before_tool(Box::new(|_ctx| Box::pin(async move { Ok(None) })))
        .after_tool(Box::new(|_ctx| Box::pin(async move { Ok(None) })))
        .close_fn(|| Box::pin(async { println!("  [lifecycle] Closed"); }))
        .build();

    assert!(plugin.on_user_message().is_some());
    assert!(plugin.on_event().is_some());
    assert!(plugin.before_run().is_some());
    assert!(plugin.after_run().is_some());
    assert!(plugin.before_agent().is_some());
    assert!(plugin.after_agent().is_some());
    assert!(plugin.before_model().is_some());
    assert!(plugin.after_model().is_some());
    assert!(plugin.before_tool().is_some());
    assert!(plugin.after_tool().is_some());
    println!("✓ Full-lifecycle plugin: all 10 hooks registered");

    // 5. Builder produces correct Debug output
    let debug = format!("{:?}", plugin);
    assert!(debug.contains("full-lifecycle"));
    assert!(debug.contains("has_on_user_message: true"));
    assert!(debug.contains("has_before_model: true"));
    println!("✓ Debug output shows hook registration status");

    println!("\n=== All builder tests passed! ===");
}
