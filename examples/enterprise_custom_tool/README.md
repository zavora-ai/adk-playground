# Enterprise Custom Tool Example

Demonstrates the full custom tool round-trip pattern with `adk-enterprise`:

1. Creates an agent with a custom `get_weather` tool (JSON schema with a `city` string parameter)
2. Creates a session
3. Opens an SSE event stream
4. Sends "What's the weather in Tokyo?"
5. Handles `CustomToolUse` events by executing the tool locally and returning results
6. Prints the agent's final text response
7. Archives the session and deletes the agent

## Prerequisites

- A valid ADK Enterprise API key

## Setup

```bash
cp .env.example .env
# Edit .env and set your ADK_API_KEY
```

## Usage

```bash
# From the workspace root:
cargo run --manifest-path examples/enterprise_custom_tool/Cargo.toml

# Or from this directory:
cargo run
```

## How It Works

The agent is configured with a custom tool named `get_weather`. When the agent decides to use this tool, it emits a `CustomToolUse` event via SSE. The client intercepts this event, executes the tool locally (returning a hardcoded weather string), and sends the result back using `custom_tool_result`. The agent then uses the result to compose its final response.

This pattern enables hybrid architectures where LLM reasoning runs on the platform while tool execution stays local (for security, latency, or access control reasons).
