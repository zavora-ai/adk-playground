//! Plugin system basics — validates PluginConfig, Plugin, and PluginManager
//!
//! Demonstrates: creating plugins with callbacks, composing a PluginManager,
//! and the lifecycle hook points available.

use adk_plugin::{Plugin, PluginConfig, PluginManager};

fn main() {
    println!("=== Plugin System Basics ===\n");

    // 1. Create a logging plugin with message and event hooks
    let logging_plugin = Plugin::new(PluginConfig {
        name: "logging".to_string(),
        on_user_message: Some(Box::new(|_ctx, content| {
            Box::pin(async move {
                println!("  [log] User message received: {} parts", content.parts.len());
                Ok(None) // Don't modify the message
            })
        })),
        on_event: Some(Box::new(|_ctx, event| {
            Box::pin(async move {
                println!("  [log] Event: {} by {}", event.id, event.author);
                Ok(None) // Don't modify the event
            })
        })),
        ..Default::default()
    });
    assert_eq!(logging_plugin.name(), "logging");
    assert!(logging_plugin.on_user_message().is_some());
    assert!(logging_plugin.on_event().is_some());
    assert!(logging_plugin.before_model().is_none()); // not set
    println!("✓ Logging plugin created with on_user_message + on_event");

    // 2. Create a metrics plugin with run lifecycle hooks
    let metrics_plugin = Plugin::new(PluginConfig {
        name: "metrics".to_string(),
        before_run: Some(Box::new(|_ctx| {
            Box::pin(async move {
                println!("  [metrics] Run starting...");
                Ok(None) // Don't skip the run
            })
        })),
        // after_run returns () (not Result)
        after_run: Some(Box::new(|_ctx| {
            Box::pin(async move {
                println!("  [metrics] Run completed");
            })
        })),
        ..Default::default()
    });
    assert_eq!(metrics_plugin.name(), "metrics");
    assert!(metrics_plugin.before_run().is_some());
    assert!(metrics_plugin.after_run().is_some());
    println!("✓ Metrics plugin created with before_run + after_run");

    // 3. Create a tool-interceptor plugin
    // Note: before_tool and after_tool take only (ctx) — 1 argument
    let tool_plugin = Plugin::new(PluginConfig {
        name: "tool-audit".to_string(),
        before_tool: Some(Box::new(|_ctx| {
            Box::pin(async move {
                println!("  [audit] Tool call about to execute");
                Ok(None) // Don't skip the tool call
            })
        })),
        after_tool: Some(Box::new(|_ctx| {
            Box::pin(async move {
                println!("  [audit] Tool call completed");
                Ok(None) // Don't modify result
            })
        })),
        // on_tool_error takes (ctx, tool, args, error_message)
        on_tool_error: Some(Box::new(|_ctx, _tool, _args, error| {
            Box::pin(async move {
                println!("  [audit] Tool error: {}", error);
                Ok(None) // Don't override error handling
            })
        })),
        ..Default::default()
    });
    assert!(tool_plugin.before_tool().is_some());
    assert!(tool_plugin.after_tool().is_some());
    assert!(tool_plugin.on_tool_error().is_some());
    println!("✓ Tool-audit plugin created with before/after/error hooks");

    // 4. Compose into a PluginManager
    let manager = PluginManager::new(vec![logging_plugin, metrics_plugin, tool_plugin]);
    println!("✓ PluginManager created with 3 plugins");

    // 5. Verify Debug output
    let debug = format!("{:?}", manager);
    assert!(debug.contains("PluginManager"));
    println!("✓ PluginManager implements Debug");

    // 6. Plugin with cleanup function
    let cleanup_plugin = Plugin::new(PluginConfig {
        name: "cleanup".to_string(),
        close_fn: Some(Box::new(|| {
            Box::pin(async {
                println!("  [cleanup] Resources released");
            })
        })),
        ..Default::default()
    });
    assert_eq!(cleanup_plugin.name(), "cleanup");
    println!("✓ Cleanup plugin with close_fn created");

    // 7. Default config has all callbacks as None
    let default_config = PluginConfig::default();
    assert_eq!(default_config.name, "unnamed");
    assert!(default_config.on_user_message.is_none());
    assert!(default_config.before_model.is_none());
    assert!(default_config.after_model.is_none());
    assert!(default_config.on_model_error.is_none());
    println!("✓ Default PluginConfig has all callbacks as None");

    println!("\n=== All plugin basics tests passed! ===");
}
