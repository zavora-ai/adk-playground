# MontyPythonCodeTool Example

An [`LlmAgent`] with a REPL-mode `MontyPythonCodeTool`: the model writes Python
that runs **in-process** via the [Pydantic Monty](https://github.com/pydantic/monty)
interpreter — no container, no subprocess.

## What it demonstrates

| Capability | How |
|------------|-----|
| Read-write filesystem mount | `/out` maps to a host temp directory; the model writes `q3.txt` through `pathlib.Path` |
| Environment grant | `PROJECT` is readable with `os.getenv`; nothing else from the host environment is exposed |
| Host function | `fx_rate(from, to)` is a Rust async function callable from Python by bare name |
| REPL persistence | Turn 2 reuses variables the model created in turn 1 — one interpreter session per ADK session |
| Composed description | The tool's LLM-facing description is rendered from the executor's own capability report (mounts, variables, functions), so prompt and behavior cannot drift |

Everything not granted — other paths, other variables, network, subprocesses —
is unreachable from Python regardless of what the model writes.

## Run

```bash
GOOGLE_API_KEY=... cargo run --manifest-path examples/monty_python_code_tool/Cargo.toml
```

Or copy `.env.example` to `.env` and set the key there.

## Expected flow

1. The example prints the composed tool description handed to the model.
2. **Turn 1** — the model stores the invoice list in a Python variable, totals
   it in EUR, converts to USD via `fx_rate`, and writes a summary line to
   `/out/q3.txt` tagged with `os.getenv("PROJECT")`.
3. **Turn 2** — the model answers the average-invoice question from the
   variables still in the REPL session, without re-entering the data.
4. The example prints the file the model wrote, read back from the real host
   directory behind the mount.
