# Harness Pattern — Trait-Based Agent Execution

A focused example of the **harness pattern**: an agent depends only on an
abstract execution contract (`Arc<dyn Harness>`), so the same agent code runs
against a real LLM in production and a deterministic double in tests — with no
conditional compilation and no mocking framework.

## The idea

```text
  Harness (trait)         │ run() / run_stream() / name()
    ├── Runner            │ Real LLM (Gemini here; any adk_core::Llm)
    ├── TestHarness       │ Canned responses, no network, no keys
    └── (your own impl)   │ Distributed, sandboxed, etc.

  MyAgent holds Arc<dyn Harness> — swappable at construction.
```

| Piece | Role |
|-------|------|
| **`Harness`** | The abstract contract: `run`, `run_stream`, `name`. |
| **`Runner`** | Production implementation — real Gemini calls via `adk_core::Llm`, with timeout and streaming. |
| **`TestHarness`** | A deterministic substitute — canned responses, no API calls or keys. |
| **`MyAgent`** | Depends **only** on `Arc<dyn Harness>` — never the concrete type. |

## What it shows

- **Dependency inversion** — `MyAgent` never imports `Runner` or `TestHarness`;
  it works with any `Harness`.
- **Real + test parity** — identical agent code drives a live Gemini run and a
  deterministic test run.
- **Streaming** — `run_stream` returns a channel of token chunks.
- **Timeout handling** — a 1 ms deadline demonstrates the `HarnessError::Timeout`
  path.

## Run

```bash
export GOOGLE_API_KEY=…          # required for the production Runner section
cargo run --manifest-path examples/harness_pattern/Cargo.toml
```

The program runs four sections: a real Gemini turn (plus a streaming turn), the
`TestHarness` returning the same canned answer regardless of input, and a forced
timeout — printing how each path behaves.
