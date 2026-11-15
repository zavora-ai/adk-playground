# Gemini Interactions API — Runtime Agent Example

This crate mirrors the ADK-Python `interactions_api` sample, driving Google's
**Interactions API (Beta)** through ADK-Rust. The key idea — exactly as in
ADK-Python — is that the Interactions API is a **transport toggle on the Gemini
model**, not a new agent type. The same `LlmAgent`, `Runner`, tool loop, and
sessions are used unchanged. The only difference is one line:

```rust
let model = GeminiModel::new(api_key, "gemini-2.5-flash")?
    .use_interactions_api(true)?; // transport toggle
```

With the toggle on, the model gains server-side history, the interactions step
model, and a first-class `interaction_id` surfaced on every `Event`.

## What this example shows

| Scenario | Demonstrates |
| --- | --- |
| 1. Basic generation | A normal `LlmAgent` on the Interactions transport, printing the response and the server-assigned `interaction_id`. |
| 2. Google Search via bypass | `GoogleSearchTool::with_bypass_multi_tools_limit(...)` converts the built-in search tool into a function-calling tool so it can coexist with custom function tools. |
| 3. Multi-turn stateful conversation | Two turns on the same session: server-side context retention and chained `interaction_id`s (turn 2 recalls a fact from turn 1). |
| 4. Custom function tool + bypassed search | The tool-mixing solution in action — a plain function tool (`celsius_to_fahrenheit`) alongside the bypassed `google_search`. |

### The tool-mixing limitation

The Interactions API rejects mixing built-in (server-side) tools such as
`google_search` with custom function tools in a single request. ADK-Python
solves this with `bypass_multi_tools_limit=True`; ADK-Rust mirrors it with
`with_bypass_multi_tools_limit`, which routes the built-in behavior through an
internal single-turn `LlmAgent` and exposes a uniform function tool. Scenarios 2
and 4 use this.

## Setup

This example requires a Gemini API key with **Interactions API (Beta)** access.

```bash
cp .env.example .env
# then edit .env and set GOOGLE_API_KEY=...
```

`GEMINI_API_KEY` is accepted as a fallback. If no key is present, the example
prints guidance and exits cleanly without making network calls.

## Run

From the workspace root:

```bash
cargo run -p gemini-interactions-agent
```

Or with an explicit manifest path:

```bash
cargo run --manifest-path examples/gemini_interactions_agent/Cargo.toml
```

Set `RUST_LOG=debug` for verbose tracing output:

```bash
RUST_LOG=debug cargo run -p gemini-interactions-agent
```

## Notes

- `gemini-2.5-flash` is used because it is on the Interactions API target
  allowlist. Enabling the transport with a non-allowlisted target returns an
  `InvalidInput` error up front.
- generateContent remains the default and recommended transport for stable
  production. The Interactions API is Beta — opt in deliberately.
- The `interaction_id` is a first-class field on `LlmResponse`/`Event`
  (`event.interaction_id()`), mirroring ADK-Python's `event.interaction_id`.
