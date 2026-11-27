# AP Desk — supplier invoices read, matched and posted, exceptions put to the clerk

An accounts payable agent over a day's inbox of forty-four supplier invoices. PP-OCRv6 reads
each page on this computer. When a field's line is uncertain or does not parse, the agent sends
Claude that line's crop, never the page. It checks for duplicates with SigLIP 2 image
embeddings and the ERP's posted invoices, matches the invoice against the supplier record,
the purchase order and its goods receipt, and posts it — or puts it on the clerk's desk with
the page as evidence and two sentences on what is wrong and what to do.

The invoices are FATURA's: synthetic, in fifty layouts, with every field annotated. The run
scores what the desk read and decided against those annotations and the policy.

| ADK-Rust piece | What it does here |
|----------------|-------------------|
| `adk-vision` `PpOcrReader` (PP-OCRv6 small) | `vision_read` on this computer: every line with its box and confidence, 0.3 s a page |
| `vision_describe` behind the `EgressGate` | Claude reads one line's crop when the line is below 0.90 confidence or its value does not parse; each crop is a recorded release of data class `document_crop` |
| `VisualMemory` with SigLIP 2 | `similar_invoices`: the earlier pages most like this one, with what the desk read from them |
| The app's ERP tools | `erp_lookup` (supplier, PO, goods receipt, posted invoices), `post_invoice` (an edit to the books, allowed), `raise_exception` (held for the clerk with the page) |
| `adk-harness` | The agent in Auto mode with the vision pack: reads and posts run, exceptions wait |
| `adk-kernel` | One `Triggered` process per invoice, in the invoice's own session, in arrival order, under a daily spending cap |
| `adk-surface` web API | The AP Desk page reads each held exception and answers it, reads each invoice's thread and sends the clerk's questions; the web home at `/ui/` is the developer view |

```text
inbox/d07.jpg ─▶ kernel process in session d07
                  ├─ vision_read ─────────▶ PP-OCRv6 on this computer: lines, boxes, confidence
                  ├─ vision_describe ─────▶ egress gate ─▶ Claude: one line's crop, when it is uncertain
                  ├─ similar_invoices ────▶ SigLIP 2 on this computer: look-alike pages and what was read
                  ├─ erp_lookup ──────────▶ supplier, PO and goods receipt, invoices already posted
                  └─ post_invoice  or  raise_exception ─▶ held on the clerk's desk, the page as evidence
```

The policy, first rule that applies: **duplicate** (an earlier invoice from the supplier with
the same number, or a look-alike page with the same supplier, total and date); **unknown
supplier**; with a PO, **over PO** (more than 2% above the PO amount) or **not received** (no
goods receipt); without one, **over limit** (above 1,000); otherwise **post**.

## Requirements

| What | Why |
|------|-----|
| Rust 1.95 or later | ADK-Rust 3.0 |
| 400 MB of disk | FATURA's archive; the inbox built from it is 2 MB |
| `ANTHROPIC_API_KEY`, for `--live` only | The agent and the crop reader (`claude-sonnet-5-5`) |

The build downloads ONNX Runtime. The first run fetches PP-OCRv6 small and SigLIP 2 base
quantized into the Hugging Face cache, each file checked against adk-vision's built-in
manifest before it loads.

```bash
cd examples/ap-desk
cargo run --release -- fetch
```

```text
fetching FATURA (Zenodo record 8261508) and building the inbox
  fetching FATURA.zip (363504027 bytes) from Zenodo record 8261508
  wrote    44 documents to data/inbox (29 to post, 15 exceptions), the ERP and the expected answers
```

[DATA.md](DATA.md) has the licence, the checksum and how the inbox and the ERP are built.

## Run it headless

```bash
cargo run --release -- run            # replays the recorded answers: no key, no spend
cargo run --release -- run --live     # Claude live; its answers are recorded again
```

Each invoice runs in its own session as the inbox is worked. A replay of the recorded day,
three invoices of it:

```text
[d06] call vision_read {"image":"d06.jpg"}
[d06] call vision_describe {"image":"d06.jpg","question":"What is the exact text on this line (the invoice date)?","region":[385,260,535,325]}
[d06] call erp_lookup {"supplier":"White and Sons"}
┌ [d06] on the clerk's desk (over_limit): Over limit: 1,171.68 without PO
│ Invoice 6Y9M9d-265 from White and Sons has no PO and its total of 1,171.68 USD is 171.68 above the 1,000 limit for invoices without a PO. I suggest the clerk confirm the purchase and approve it, or ask the supplier for a PO.
┌ [d11] on the clerk's desk (over_po): Over PO 35 by 13.6%
│ The invoice total of 832.93 USD is 99.95 (13.6%) above the PO 35 amount of 732.98 USD, beyond the 2% tolerance; the goods receipt GR-40001 exists. …
┌ [d21] on the clerk's desk (duplicate): Duplicate of d03 INV82103976
│ Invoice INV82103976 from Phillips, Thomas and Malone for 694.91 USD dated 1993-06-11 was already posted as d03 (JE-70004), and this page matches it at 0.97 similarity. …
…
results (44 documents in 40s)
  supplier    43 of 44 right
  number      44 of 44 right
  date        44 of 44 right
  po          44 of 44 right
  total       44 of 44 right
  currency    44 of 44 right
  every field right, clean         31 of 31
  every field right, phone photo   8 of 9
  every field right, re-sent scan  4 of 4
  decisions right                43 of 44
  exceptions caught              15 of 15 the policy calls for; 0 clean invoices held, 0 posted that should not be
    expected duplicate         → duplicate 4
    expected not_received      → not_received 2
    expected over_limit        → over_limit 2
    expected over_po           → over_po 3
    expected post              → post 29
    expected unknown_supplier  → unknown_supplier 3, unreadable 1
  crops sent to Claude           8
  agent  claude-sonnet-5-5   165 calls   1586342 in   28361 out  $1.4018
  crop   claude-sonnet-5-5     8 calls       606 in    1642 out  $0.0176
  total model spend              $1.4195 (the recorded live run's; this replay spent nothing)
```

### What the numbers say

| Measure | Result |
|---------|--------|
| Fields read right (supplier, number, date, PO, total, currency) | 263 of 264 |
| Invoices with every field right | 31 of 31 rendered, 8 of 9 phone photos, 4 of 4 re-sent scans |
| Decisions as the policy gives them | 43 of 44 |
| Exceptions caught | 15 of 15; no clean invoice held, nothing posted that should not be |
| Duplicates caught | 4 of 4, each naming the posting it repeats |
| Crops sent to Claude | 8, all from phone photos; no whole page left the machine |
| Cost of the live day | $1.42, almost all of it the agent's turns; the eight crops cost $0.02 |

- **Reading stays on the device.** PP-OCRv6 small read every rendered page and every re-sent
  scan without help. On the phone photos it misread a few characters — `31-Ocs-2011` for
  `31-Oct-2011` — and the agent sent Claude only those lines.
- **The one miss** is d10, a phone photo from the supplier the ERP does not know. That layout
  prints the seller's name below the dates rather than at the top, the agent took it for part of
  the buyer's block, and it raised the invoice as unreadable instead of as an unknown supplier.
  It still reached the clerk.
- **Visual memory proposes, the books decide.** Re-sent copies score 0.94–0.97 against their
  originals, but pages from other suppliers reach 0.90, so a score alone would not do. The agent
  compares the look-alike's supplier, number, total and date before calling it a duplicate.
- **Totals are matched as printed.** FATURA's subtotal, discount and tax rarely add up to its
  total, so the policy does not treat that as an exception; [DATA.md](DATA.md) has the count.

| Flag | Default | Meaning |
|------|---------|---------|
| `--live` | off | Calls Claude and records its answers to `recordings/tape.json`; without it the recording replays |
| `--docs` | all 44 | Works only the first documents of the inbox |
| `--cards` | `hold` | An exception held for the clerk: `hold` leaves it on the desk, `approve` accepts it, `deny` declines it |
| `--agent-model` | `claude-sonnet-5-5` | The agent |
| `--crop-model` | `claude-sonnet-5-5` | What `vision_describe` sends a crop to |
| `--daily-usd` | `5.00` | The kernel's daily spending cap |
| `--out` | `out` | The journal, accepted exceptions and `results.json` |

## Run it in the browser

```bash
cargo run --release -- serve            # replayed: no key, no spend
cargo run --release -- serve --live     # live; Claude's answers are recorded again
```

Open `http://127.0.0.1:8817/ap`. The page is the whole desk, and nothing is read until the
clerk presses **Start the day**.

| Part | What it shows and does |
|------|------------------------|
| Header | `Replay` or `Live`; the recorded spend ("$1.42 when it was recorded") or this run's against the daily cap ("$0.26 of $5.00"); "31 of 44 invoices worked"; **Start the day**, which reads "Day finished" once the inbox is worked |
| Inbox | Every invoice in arrival order: waiting to be read, being read, posted, or waiting for the clerk, with its supplier and amount once read. "3 waiting for you" opens the next one |
| What AP Desk read | The page, the fields read from it, the crops sent to Claude, and the match against the ERP: supplier, PO and goods receipt, the total against the PO or the 1,000 limit, an earlier posting with the same number, the most alike page |
| Your decision | The invoice's exception: its kind, the mismatch against the PO or the ERP in plain words, the line of the page it is about, and AP Desk's suggestion. **Approve** files `out/exceptions/<doc>.json`; **Decline** takes an optional reason, which AP Desk reads. Once answered it collapses to "You approved at 08:25" |
| Conversation | The clerk's messages and AP Desk's replies; each turn's tool steps as one line ("Worked for 4 s · 6 steps"), and a failed turn's reason ("Claude could not answer: …") |
| Ask box | "Ask about d07…": a question about that invoice, answered in the conversation while the box says "AP Desk is answering…" |

In a replay, decisions work and the ask box is off: questions need a live run. The turn after a
decision was never recorded, so AP Desk answers it with "This is a replay, and this answer was
not recorded. Run with --live for a real answer."; `run` keeps a missing answer an error, since
there it is the determinism check. **Developer view**, in the footer, opens the ADK-Rust web
home at `/ui/`.

## What leaves the machine

| Data | Where it goes |
|------|---------------|
| Pages | Nowhere. PP-OCRv6 and SigLIP 2 run on this computer |
| A line's crop, when it is uncertain | `api.anthropic.com`, through the egress gate as `document_crop`, each release recorded |
| The text PP-OCRv6 read, the ERP's answers | The agent model, as text |

Every restriction is an opt-in, and this example sets none. With `out/.adk/vision.toml` holding
`[egress] default = "ask"`, each crop waits for the clerk with the exact image it would send;
`deny` keeps every crop on this computer, and the agent then raises the field as unreadable.

## Files

| File | Holds |
|------|-------|
| `src/main.rs` | The commands, working the inbox, and the report |
| `src/data.rs` | FATURA's fetch and checksum, the inbox and its scans, the ERP and the expected answers |
| `src/desk.rs` | The harness agent with the vision pack, the visual memory, the four tools and the kernel |
| `src/eval.rs` | Fields and decisions scored against the annotations and the policy |
| `src/tape.rs` | Claude's answers recorded live and replayed; in the browser, an unrecorded agent turn answered as a replay |
| `src/terminal.rs` | Each invoice's turn as it happens, and exceptions decided by `--cards` |
| `src/serve.rs` | The AP Desk page and its routes: the day, each invoice's detail and the line of the page an exception is about |
