# ADK Playground

Examples, playground, and documentation validation for [ADK-Rust](https://github.com/zavora-ai/adk-rust) — the Rust Agent Development Kit.

## What's here

| Directory | Description |
|-----------|-------------|
| `examples/` | 170+ runnable examples organized by provider and feature |
| `playground/` | Web-based playground with live code editor, streaming output, and execution traces |
| `docs_examples/` | Compilable code snippets that validate the official documentation |

## Quick start

```bash
# Gemini quickstart (default provider)
GEMINI_API_KEY=your_key cargo run --example quickstart

# OpenAI
OPENAI_API_KEY=your_key cargo run --example openai_basic --features openai

# Anthropic
ANTHROPIC_API_KEY=your_key cargo run --example anthropic_quickstart --features anthropic

# DeepSeek
DEEPSEEK_API_KEY=your_key cargo run --example deepseek_quickstart --features deepseek

# xAI (Grok)
XAI_API_KEY=your_key cargo run --example xai_quickstart --features xai

# Local models via Ollama
cargo run --example ollama_basic --features ollama
```

## Playground

The playground is a web app for running ADK-Rust examples in the browser with streaming output, execution traces, token usage, and cost estimates.

```bash
# Build the frontend
cd playground/frontend && npm install && npm run build && cd ../..

# Start the backend (serves frontend + runs examples)
cd playground/backend && cargo run --release
```

Then open http://localhost:9876.

The playground includes 46 curated examples covering agents, tools, thinking/reasoning, workflows, sessions, RAG, and more.

## Examples by category

**Providers** — OpenAI, Anthropic, Google Gemini, DeepSeek, xAI/Grok, Groq, Mistral, Azure AI, AWS Bedrock, Ollama, mistral.rs (local)

**Agents** — LLM agents, graph agents, multi-agent systems, supervisor routing, customer service pipelines

**Thinking/Reasoning** — Extended thinking with Anthropic, DeepSeek, Gemini, OpenAI, and xAI

**Tools** — Function tools, MCP (stdio + HTTP + OAuth), tool macros, agent-as-tool, multi-turn tool use, toolset composition

**Workflows** — Sequential, parallel, conditional routing, loop refinement, graph workflows

**RAG** — Basic retrieval, custom embedders, multi-collection, reranking, SurrealDB, markdown chunking

**Sessions** — In-memory, PostgreSQL, Redis, MongoDB, Neo4j, SQLite

**Realtime** — WebSocket audio streaming, VAD, tool use during realtime sessions, agent handoff

**Evaluation** — Agent eval, trajectory eval, semantic similarity, rubric scoring, LLM-as-judge

**Deployment** — A2A protocol, REST servers, CLI launcher, session compaction

**UI** — React clients, UI protocol profiles, server-rendered agents

**Security** — RBAC, JWT auth, OIDC, SSO, Google OAuth, guardrails, schema validation

**Other** — Artifacts, callbacks, code execution, memory, plugins, skills, structured output, telemetry

## Standalone crates

Some examples are standalone crates with their own dependencies:

```bash
cargo run -p ralph                  # Multi-agent research assistant
cargo run -p audio                  # Audio pipeline (STT/TTS)
cargo run -p postgres_session       # PostgreSQL session store
cargo run -p redis_session          # Redis session store
cargo run -p mongodb_session        # MongoDB session store
cargo run -p neo4j_session          # Neo4j session store
cargo run -p sqlite_memory          # SQLite memory store
```

## Environment variables

Create a `.env` file in `examples/` or set these in your shell:

```
GEMINI_API_KEY=...
OPENAI_API_KEY=...
ANTHROPIC_API_KEY=...
DEEPSEEK_API_KEY=...
XAI_API_KEY=...
GROQ_API_KEY=...
MISTRAL_API_KEY=...
```

## Requirements

- Rust 1.85+ (edition 2024)
- Node.js 18+ (for the playground frontend)
- API keys for the providers you want to use

## Request an example

Want to see a specific example? [Open an issue](https://github.com/zavora-ai/adk-rust/issues/new?template=example_request.yml) using the Example Request template.

## License

Apache-2.0
