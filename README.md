# ADK Playground

Examples and documentation validation code for [ADK-Rust](https://github.com/zavora-ai/adk-rust).

## Structure

- `examples/` — 150+ runnable examples organized by provider and feature
- `docs_examples/` — Compilable code snippets validating official documentation

## Quick Start

```bash
# Run the quickstart example (Gemini)
GEMINI_API_KEY=your_key cargo run --example quickstart

# Run an OpenAI example
OPENAI_API_KEY=your_key cargo run --example openai_basic --features openai

# Run a standalone crate example
cargo run -p ralph
```

## Dependencies

All ADK crates are pulled from the [adk-rust](https://github.com/zavora-ai/adk-rust) git repository.
