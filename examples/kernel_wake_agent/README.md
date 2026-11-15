# Kernel Wake Agent — an agent an event starts

An autonomous inbox agent on the agent kernel. No prompt starts it: a file landing in
`inbox/` does, and the agent's turn begins with the event that woke it.

## What This Shows

- **`Kernel`** — runs agents as durable processes over one SQLite file
- **`Trigger`** — a standing wake; each matching event starts one process
- **`Wake::FileChange`** — the event source: a settled change under a watched directory
- **The wake notice** — the kernel adds the event (source, key, time, payload) to the
  turn as delimited user content, so the agent learns *which* file changed without a
  fixed prompt naming it
- **Confined tools** — the path in a wake payload comes from outside the agent, so
  `read_inbox_file` reads only inside `inbox/` and `write_note` writes only inside `notes/`

## Flow

| Step | Who | What happens |
|------|-----|--------------|
| 1 | Any program | Saves `inbox/disk-alert.txt` |
| 2 | Kernel | The file source emits an event keyed `file:<root>:<path>:<modified>`; the `inbox` trigger starts one process for it |
| 3 | Kernel | The process's turn starts with its own input plus the wake notice carrying `{"event", "path", ...}` |
| 4 | Agent | Calls `read_inbox_file` with the path from the notice, then `write_note` |
| 5 | Agent | `notes/disk-alert.md` holds the summary, urgency and next action |

A save of the same contents at the same modification time is the same event key, so a
redelivered event starts nothing.

## Prerequisites

- **Rust 1.95+** (edition 2024)
- **`GOOGLE_API_KEY`** — optional. With it the agent is `gemini-3.7-flash`. Without it the
  example uses `offline::RuleModel`, fixed rules that read the same request a model
  receives and answer with the same tool calls, so the whole path runs with no network.

## Run

```bash
cargo run --manifest-path examples/kernel_wake_agent/Cargo.toml
```

The demo saves two sample files, waits for the agent's notes and prints them:

```text
model: offline rules (set GOOGLE_API_KEY for Gemini)
watching /.../wake-agent/inbox
saved inbox/disk-alert.txt
saved inbox/lunch.txt

--- notes/disk-alert.md ---
# disk-alert.txt

- urgency: urgent
- summary: URGENT: db-03 is at 97% disk and the nightly backup failed twice.
- next action: page the on-call owner

--- notes/lunch.md ---
# lunch.txt

- urgency: routine
- summary: Team lunch moves to Thursday 12:30. Reply if you cannot make it.
- next action: read at the next review
```

To leave it running and feed it yourself:

```bash
cargo run --manifest-path examples/kernel_wake_agent/Cargo.toml -- --watch
```

Then, from another terminal, save any `.txt` file into `wake-agent/inbox/`. Ctrl-C drains
and stops the kernel.

| Variable | Default | Purpose |
|----------|---------|---------|
| `GOOGLE_API_KEY` | unset | Uses Gemini instead of the offline rules |
| `WAKE_AGENT_HOME` | `wake-agent` | Holds `inbox/`, `notes/` and `kernel.db` |
| `RUST_LOG` | `error` | `RUST_LOG=adk_kernel=debug,adk_server=info` shows each wake being routed and claimed |

## ADK-Rust revision

The wake notice is added by [adk-rust#1381](https://github.com/zavora-ai/adk-rust/pull/1381),
so this example pins that pull request's commit rather than the revision the other
examples share. Move it to the shared revision once that revision includes the change.

> **Note:** The kernel starts its file watcher on a task of its own after `Kernel::start`
> returns, and a trigger sees only the changes made while it watches. The demo waits one
> second before saving its sample files for that reason.
