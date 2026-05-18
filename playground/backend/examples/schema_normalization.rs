use adk_core::{GenericSchemaAdapter, SchemaAdapter, SchemaCache};
use adk_gemini::schema_adapter::GeminiSchemaAdapter;
use adk_model::anthropic::AnthropicSchemaAdapter;
use adk_model::openai::{OpenAiSchemaAdapter, OpenAiStrictSchemaAdapter};
use serde_json::json;

// ── Provider-Aware Schema Normalization (v0.8.2) ──
// Demonstrates how ADK-Rust normalizes MCP tool schemas per-provider:
//
// Each LLM provider has different JSON Schema requirements for function-calling.
// ADK normalizes schemas at request time using provider-specific adapters,
// so MCP tools work seamlessly across all providers without manual tweaking.
//
// No API keys needed — this is pure schema transformation logic.

fn demo_adapter(name: &str, adapter: &dyn SchemaAdapter, schema: &serde_json::Value, notes: &[&str]) {
    println!("━━━ {} ━━━\n", name);
    for note in notes {
        println!("  {note}");
    }
    println!();
    let normalized = adapter.normalize_schema(schema.clone());
    println!("{}\n", serde_json::to_string_pretty(&normalized).unwrap());
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== Provider-Aware Schema Normalization (v0.8.2) ===\n");

    // A complex MCP tool schema with features that providers handle differently
    let raw_schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "definitions": {
            "Address": {
                "type": "object",
                "properties": {
                    "street": { "type": "string" },
                    "city": { "type": "string" },
                    "zip": { "type": "string", "format": "postal-code" }
                },
                "additionalProperties": false
            }
        },
        "properties": {
            "name": { "type": ["string", "null"], "description": "Customer name" },
            "email": { "type": "string", "format": "email" },
            "age": { "type": "integer", "exclusiveMinimum": 0, "exclusiveMaximum": 150 },
            "address": { "$ref": "#/definitions/Address" },
            "status": { "const": "active" },
            "tags": {
                "type": "array",
                "items": { "type": "string", "format": "hostname" }
            },
            "metadata": {
                "anyOf": [
                    { "type": "null" },
                    { "type": "object", "additionalProperties": true }
                ]
            }
        },
        "required": ["name", "email"],
        "additionalProperties": false,
        "if": { "properties": { "status": { "const": "active" } } },
        "then": { "required": ["address"] }
    });

    println!("── Raw MCP Tool Schema ──\n");
    println!("{}\n", serde_json::to_string_pretty(&raw_schema).unwrap());

    // --- Gemini ---
    demo_adapter(
        "Gemini (Standard)",
        &GeminiSchemaAdapter::new(),
        &raw_schema,
        &[
            "• Resolves $ref → inlines Address definition",
            "• Collapses anyOf → picks first non-null sub-schema",
            "• Collapses type arrays → [\"string\", \"null\"] becomes \"string\"",
            "• Removes: $schema, additionalProperties, exclusiveMin/Max, if/then",
            "• Converts const → single-element enum",
        ],
    );

    // --- OpenAI Strict ---
    demo_adapter(
        "OpenAI (Strict Mode)",
        &OpenAiStrictSchemaAdapter,
        &raw_schema,
        &[
            "• Preserves $ref and definitions (OpenAI supports them)",
            "• Preserves anyOf (nullable patterns)",
            "• Adds additionalProperties: false to ALL object schemas",
            "• Strips: $schema, if/then/else",
            "• Converts const → enum",
        ],
    );

    // --- OpenAI Non-Strict ---
    demo_adapter(
        "OpenAI (Non-Strict)",
        &OpenAiSchemaAdapter,
        &raw_schema,
        &[
            "• Minimal safe fixes only",
            "• Preserves $ref, anyOf, additionalProperties, type arrays",
            "• Strips: $schema, if/then/else",
        ],
    );

    // --- Anthropic ---
    demo_adapter(
        "Anthropic",
        &AnthropicSchemaAdapter,
        &raw_schema,
        &[
            "• Near pass-through — Anthropic supports most JSON Schema",
            "• Preserves: $ref, definitions, anyOf, additionalProperties, const, ALL formats",
            "• Only strips: $schema, if/then/else",
        ],
    );

    // --- Generic ---
    demo_adapter(
        "Generic (Ollama, etc.)",
        &GenericSchemaAdapter,
        &raw_schema,
        &[
            "• Conservative safe transforms for unknown providers",
            "• Strips: $schema, if/then/else",
            "• Converts const → enum",
            "• Does NOT resolve $ref or collapse combiners",
        ],
    );

    // --- Tool Name Truncation ---
    println!("━━━ Tool Name Truncation ━━━\n");

    let long_name = "mcp_server_github_com_organization_repository_pull_request_review_comments_list_all";
    let emoji_name = "🔧_tool_with_emoji_名前が長いツール_herramienta_larga";

    let adapters: Vec<(&str, Box<dyn SchemaAdapter>)> = vec![
        ("Gemini", Box::new(GeminiSchemaAdapter::new())),
        ("OpenAI", Box::new(OpenAiSchemaAdapter)),
        ("Anthropic", Box::new(AnthropicSchemaAdapter)),
    ];

    for (provider, adapter) in &adapters {
        let truncated = adapter.normalize_tool_name(long_name);
        println!("  {provider:12} │ \"{}...\" → \"{}\" ({} bytes)",
            &long_name[..40], truncated, truncated.len());
    }

    println!();
    for (provider, adapter) in &adapters {
        let truncated = adapter.normalize_tool_name(emoji_name);
        println!("  {provider:12} │ \"{}\" → \"{}\" ({} bytes)",
            emoji_name, truncated, truncated.len());
    }

    // --- Schema Cache ---
    println!("\n━━━ Schema Cache ━━━\n");

    let cache = SchemaCache::new();
    let adapter = GeminiSchemaAdapter::new();

    println!("  Cache empty: {} entries", cache.len());
    let _r1 = cache.get_or_normalize(&raw_schema, &adapter);
    println!("  After first normalize: {} entry (computed)", cache.len());
    let _r2 = cache.get_or_normalize(&raw_schema, &adapter);
    println!("  After second normalize: {} entry (cache hit!)", cache.len());

    let different = json!({"type": "string", "format": "email"});
    let _r3 = cache.get_or_normalize(&different, &adapter);
    println!("  After different schema: {} entries", cache.len());

    println!("\n=== Key Features ===");
    println!("• SchemaAdapter trait — common interface for per-provider normalization");
    println!("• GeminiSchemaAdapter — destructive transforms (resolves $ref, collapses combiners)");
    println!("• OpenAiStrictSchemaAdapter — preserves structure, adds additionalProperties: false");
    println!("• AnthropicSchemaAdapter — near pass-through (most JSON Schema supported)");
    println!("• GenericSchemaAdapter — conservative default for unknown providers");
    println!("• SchemaCache — thread-safe cache keyed by content hash");
    println!("• Tool name truncation at valid UTF-8 boundaries (64 byte limit)");
    println!("• MCP tools now work seamlessly across ALL providers without manual tweaking");
    Ok(())
}
