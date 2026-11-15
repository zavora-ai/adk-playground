# Enterprise Hello World

The simplest possible ADK Enterprise managed agent session. Demonstrates the full lifecycle: create an agent, start a session, stream responses, and clean up.

## Prerequisites

- An ADK Enterprise API key (get one at <https://enterprise.adk-rust.com>)

## Setup

```bash
cp .env.example .env
# Edit .env and add your API key
```

Or export directly:

```bash
export ADK_API_KEY=adk_live_...
```

## Run

```bash
cargo run -p enterprise-hello
```

## What it does

1. Creates an `EnterpriseClient` from the `ADK_API_KEY` environment variable
2. Creates an agent configured with `gemini-2.5-flash` and a brief system prompt
3. Creates a session on that agent
4. Opens an SSE event stream (must happen before sending messages)
5. Sends "What is 2+2?" to the agent
6. Prints text content from `Message` events as they arrive
7. Breaks when `StatusIdle` indicates the turn is complete
8. Archives the session and deletes the agent

## Self-Hosted

To point at your own deployment instead of the hosted platform:

```rust
let client = EnterpriseClient::self_hosted(
    "adk_live_...",
    "https://your-server.internal/managed/v1",
)?;
```
