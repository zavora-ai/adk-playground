# Schema Normalization Demo

Demonstrates ADK-Rust's provider-aware schema normalization — the architecture that lets MCP tools work seamlessly across all LLM providers without manual schema tweaking.

## What It Shows

Each LLM provider has different JSON Schema requirements for function-calling:

| Provider | Behavior |
|----------|----------|
| **Gemini** | Destructive transforms: resolves `$ref`, collapses combiners, removes unsupported keywords |
| **Gemini (Vertex AI)** | Same as Gemini but sets `additionalProperties: false` instead of removing it |
| **OpenAI Strict** | Preserves structure, adds `additionalProperties: false` everywhere |
| **OpenAI Non-Strict** | Minimal safe fixes only |
| **Anthropic** | Near pass-through — only strips `$schema` and conditional keywords |
| **Generic** | Conservative transforms for unknown providers (Ollama, etc.) |

The demo takes a single complex MCP tool schema and shows how each adapter normalizes it differently.

## Architecture

```
MCP Server → raw schema → McpToolset (stores verbatim)
                                ↓
                    Model.generate_content()
                                ↓
                    SchemaAdapter.normalize_schema()
                                ↓
                    Provider API (Gemini/OpenAI/Anthropic)
```

Schema normalization happens at **request time**, not at tool registration. This means:
- The same tool works with any provider
- No information is lost during tool discovery
- Each provider gets exactly the schema format it needs

## Run

```bash
cargo run -p schema-normalization-example
```

No API keys needed — this example only demonstrates the normalization logic locally.

## Key Concepts

### SchemaAdapter Trait

```rust
pub trait SchemaAdapter: Send + Sync + Debug {
    fn normalize_schema(&self, schema: Value) -> Value;
    fn normalize_tool_name<'a>(&self, name: &'a str) -> Cow<'a, str>;
    fn empty_schema(&self) -> Value;
}
```

### SchemaCache

Normalized schemas are cached by content hash to avoid redundant computation:

```rust
use adk_core::{GenericSchemaAdapter, SchemaCache};
use serde_json::json;
use std::sync::Arc;

let cache = SchemaCache::for_adapter(Arc::new(GenericSchemaAdapter));
let raw_schema = json!({"type": "object"});
let normalized = cache.normalize(&raw_schema);
// Second call returns cached result
let cached = cache.normalize(&raw_schema);
assert_eq!(normalized, cached);
```

### Tool Name Truncation

All adapters truncate tool names to 64 bytes at valid UTF-8 character boundaries:

```rust
let adapter = GeminiSchemaAdapter::new();
let name = "very_long_mcp_tool_name_that_exceeds_sixty_four_bytes_limit_here";
let truncated = adapter.normalize_tool_name(name); // ≤ 64 bytes, valid UTF-8
```
