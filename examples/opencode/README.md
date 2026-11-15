# OpenCode Go and Zen

Streams one reply from an OpenCode Go model through `OpenCodeClient`, which selects the wire API
from the model ID and sends the application's `User-Agent` and `x-opencode-session` headers.

## Prerequisites

- `OPENCODE_API_KEY` for an OpenCode Go account. Running the example uses that account's quota.

## Run

```bash
cd examples/opencode
cp .env.example .env   # add your OPENCODE_API_KEY
cargo run
```

## Notes

- The application supplies its own user agent and a stable conversation ID. Reuse the ID for
  auxiliary requests in the same conversation and start a new one for each conversation.
- To use Zen, change the service to `OpenCodeService::Zen` and supply a key with Zen access. Zen
  usage is billed separately from Go.
- Set `RUST_LOG=debug` for request-level tracing.

See the [OpenCode guide](https://github.com/zavora-ai/adk-rust/blob/main/docs/official_docs/models/opencode.md) for the routing table and
configuration options.
