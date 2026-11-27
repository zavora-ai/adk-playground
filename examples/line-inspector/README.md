# Line Inspector — every candle on the line inspected, borderline parts given a second look

A quality agent over candle line 2: trays of four tea lights come down the conveyor and a
camera over it sees each one. On this computer every candle is scored against known-good
candles with SigLIP 2. A clear defect is rejected, and its region is segmented and measured in
millimetres, with the candle as the ruler. A borderline part wakes the agent, which asks two
models about the part's crop: when they agree it decides, and when they disagree it puts the
part, both opinions and the size on the quality engineer's desk.

The trays are VisA's candle class, photographs with every defect masked, so the run scores the
line against those masks.

| ADK-Rust piece | What it does here |
|----------------|-------------------|
| `adk-vision` `StreamPipeline` with a `FrameSource` | The camera over the conveyor: one tray a frame, through the same pipeline a live camera uses |
| A custom `Detector` on `VisionRuntime` | The inspector: each candle found on the dark mat, cut into nine overlapping tiles, each tile embedded with SigLIP 2 and compared with the same tile of 160 known-good candles; a candle scores as its least familiar tile |
| `Sam2` (SAM 2.1 Tiny) and `Calibration` | A reject's defect, found by its colour against the candle's own wax inside the least familiar tile, refined with SAM 2.1 and measured in millimetres against the tea light's 38 mm diameter |
| `adk-kernel` | One `Triggered` process per borderline part, in its own session |
| `adk-harness` vision pack | `vision_describe` with two backends — `claude` (Claude Sonnet, the reasoner) and `second` (Claude Haiku, the second opinion) — and `vision_measure` on the device |
| Governed tools | `part` reads, `decide_part` passes or rejects, `ask_engineer` is held for the engineer with the tray as evidence |
| `adk-surface` web API | The line page reads each part's decision and thread and sends its answers and questions through `/api/surface`; the web home at `/ui/` is the developer view |

```text
tray ─▶ StreamPipeline ─▶ inspector (on this computer): 4 candles × 9 tiles × SigLIP 2 vs known-good tiles
          score < 0.45 ─────▶ pass
          score ≥ 0.75 ─────▶ reject ─▶ defect region ─▶ SAM 2.1 ─▶ mm against the 38 mm candle ─▶ reject log
          in between ───────▶ kernel process in session t017-p2
                                ├─ part
                                ├─ vision_describe backend "claude" ─▶ egress gate ─▶ Claude Sonnet: the part's crop
                                ├─ vision_describe backend "second" ─▶ egress gate ─▶ Claude Haiku: the same crop
                                └─ agree: decide_part        disagree: vision_measure, then ask_engineer (held)
```

The thresholds come from thirty more good trays: pass below the 97th percentile of their
candles' scores, reject above the highest. The board's scale puts them at 0.45 and 0.75.

## Requirements

| What | Why |
|------|-----|
| Rust 1.95 or later | ADK-Rust 3.0 |
| 300 MB of disk | Candle's 129 MB and the trays unpacked from it |
| `ANTHROPIC_API_KEY`, for `--live` only | The agent and the reasoner (`claude-sonnet-5-5`), the second opinion (`claude-haiku-5-5`) |

The build downloads ONNX Runtime; the first run fetches SigLIP 2 base quantized and SAM 2.1
Tiny q4 into the Hugging Face cache, each file checked against adk-vision's built-in manifest.
That first run also builds the bank and the calibration, about five minutes on a laptop CPU,
and caches them in `data/bank.json`.

```bash
cd examples/line-inspector
cargo run --release -- fetch
```

```text
fetching VisA's candle class
  fetching candle: bytes 0–129238015 of VisA_20220922.tar
  unpacked 1201 files: 1,000 good trays, 100 with a defect and their masks
```

[DATA.md](DATA.md) has the licence, the byte range and its checksum.

## Run it headless

```bash
cargo run --release -- run            # replays the recorded answers: no key, no spend
cargo run --release -- run --live     # Claude live; its answers are recorded again
```

A replay of the recorded shift, lines from it:

```text
running 80 trays down line 2: pass below 0.035, reject above 0.047 (0.45 and 0.75 on the board's scale); replaying recorded answers
[reject] t001-p1 score 1.00: 5.5 mm across, 14 mm²
[second look] t001-p4 score 0.51
[agent] t001-p4 pass: Both models found no defect (0.6 and 0.8 confidence); the bent wick is normal.
┌ [t002-p3] on the engineer's desk: Wax defect near wick on t002-p3?
│ reasoner: Claude: defect, 0.6 confidence. Slot-like groove or hole in the wax around the wick, with a yellowish streak below it.
│ second:   Second model: no defect, 0.8 confidence. Wax is uniform, foil edge intact, and the dark spot at the wick is normal charring.
…
results (80 trays, 320 candles, in 416s)
  defective candles (VisA's masks)  43; good 277
    on this computer: good      borderline 20
    on this computer: good      pass       254
    on this computer: good      reject     3
    on this computer: defective borderline 14
    on this computer: defective pass       14
    on this computer: defective reject     15
    second look: engineer         9
    second look: pass (right)     13
    second look: pass (wrong)     5
    second look: reject (right)   5
    second look: reject (wrong)   2
  defects stopped (rejected or with the engineer)  24 of 43 (recall 0.56)
  good candles stopped                             10 of 277 (false-reject rate 0.036)
  defect masks (SAM 2.1 vs VisA)    mean IoU 0.38 over 11 rejects
  defect sizes                      0.4–9.2 mm across
  agent    claude-sonnet-5-5   151 calls  1319048 in  39267 out  $0.9921
  reasoner claude-sonnet-5-5    37 calls    23522 in   5771 out  $0.1119
  second   claude-haiku-5-5     40 calls    25431 in  11645 out  $0.0086
  total model spend                 $1.1126 (the recorded live run's; this replay spent nothing)
```

### What the numbers say

| Measure | Result |
|---------|--------|
| Defective candles stopped (rejected, or on the engineer's desk) | 24 of 43 (recall 0.56) |
| Good candles stopped | 10 of 277 (false-reject rate 0.036) |
| By defect | chunk of wax missing 10 of 11; damaged corner of the foil 12 of 23; deformed wick 2 of 9 |
| Second looks | 34 parts: 18 decided right, 7 decided wrong, 9 put to the engineer |
| Defect masks against VisA's | mean IoU 0.38 over the 11 rejects with a defect |
| Cost of the live shift | $1.11; the inspection itself is free, on this computer |

- **SigLIP 2 sees missing wax, not small wicks.** A chunk gone from a candle changes a whole
  tile and scores high; a deformed wick or a nick in the foil's corner often does not, and
  fourteen defective candles scored below the pass threshold. A detector trained on the line's
  own defects would close that gap; adk-vision loads one through a user manifest.
- **The second look keeps the line from over-rejecting.** Twenty good candles were borderline
  on this computer; the two models passed most of them, citing the wick or the wax as normal,
  and sent the ones they disagreed on to the engineer.
- **Sizes rest on the candle.** Each reject's region is measured against the tea light's 38 mm
  diameter; SAM 2.1 refines the region the colour search finds and is overruled when it takes
  in the whole candle.
- **The pace.** SigLIP 2 runs on the CPU here, about five seconds a tray for its 36 tiles.
  adk-vision's CoreML execution provider does not run SigLIP 2's graph on this machine, so the
  inspection does not use it.

| Flag | Default | Meaning |
|------|---------|---------|
| `--live` | off | Calls Claude and records its answers to `recordings/tape.json`; without it the recording replays |
| `--trays` | all 80 | Runs only the first trays of the line |
| `--cards` | `hold` | A part put to the engineer: `hold` leaves it on the desk, `approve` accepts the agent's suggestion, `deny` declines it |
| `--agent-model` | `claude-sonnet-5-5` | The agent |
| `--reasoner-model` | `claude-sonnet-5-5` | `vision_describe` backend `claude` |
| `--second-model` | `claude-haiku-5-5` | `vision_describe` backend `second` |
| `--daily-usd` | `5.00` | The kernel's daily spending cap |
| `--out` | `out` | The reject log and `results.json` |

## Run it in the browser

```bash
cargo run --release -- serve            # replayed: no key, no spend
cargo run --release -- serve --live     # live: the model answers questions as you ask them
```

Open `http://127.0.0.1:8818/line`. The page is the whole app; nothing runs until you press
**Start the line**.

| Part of the page | What it shows and does |
|------------------|------------------------|
| Header | The mode — **Replay** ("a recorded shift played back: no key, no spend", with the recorded spend) or **Live** (this run's spend against `--daily-usd`) — the trays inspected ("34 of 80 trays"), **Start the line**, and how many parts wait for your decision. After the last tray the button reads **Shift finished**: restart the app to run it again |
| The line | The latest tray, each candle ringed green when it passed, red when rejected and amber on a second look; before the first tray, a note in its place |
| This shift | Trays, rejects, second looks and the parts waiting for you |
| Reject rate, every ten trays | One bar per ten trays on a 0–100% axis, each labelled with its trays and its rate |
| Reject log | Every reject and second look, newest first, the parts waiting for you on top; choose a row to open the part |
| The part | What the line found — the candle cut from its tray with its least familiar area boxed in amber and a reject's defect in red, the score, the size, Line Inspector's verdict and VisA's label — then **Your decision**, the part's conversation and **Ask about t002-p3…** |

**Your decision** appears when the two models disagree on a part: Line Inspector's
suggestion, the candle, both models' opinions with their confidence, the measured size and
the score. **Approve** accepts the suggestion; **Decline** overrules it, with an optional
reason that goes to the agent. The card then reads "You approved at 08:26" or "You declined
at 08:26: <reason>", and the agent's closing reply follows in the conversation.

The conversation shows your questions and Line Inspector's replies; its tool calls fold into
one line per turn ("Worked for 4 s · 5 steps"). Each part is its own session, named by the
part's id (`t002-p3`), so a question about a part is asked in that part's thread. In a
replay the ask box is disabled — questions need a live run — and an answer the recording
does not hold, such as the reply after your decision, reads "This is a replay, and this
answer was not recorded. Run with --live for a real answer." A headless `run` keeps such a
miss an error, since it checks that the replay is exact.

**Developer view** in the footer opens the ADK-Rust web home at `/ui/`, with every thread,
decision and run.

## What leaves the machine

| Data | Where it goes |
|------|---------------|
| Every tray | Nowhere. Inspection, segmentation and measurement run on this computer |
| A borderline part's crop | `api.anthropic.com`, twice — the reasoner and the second opinion — through the egress gate, each release recorded |
| Scores, boxes and sizes | The agent model, as text |

Every restriction is an opt-in, and this example sets none. `out/.adk/vision.toml` with
`[egress] default = "ask"` holds each crop for a person with the exact image it would send.

## Files

| File | Holds |
|------|-------|
| `src/main.rs` | The commands, the run and its report |
| `src/data.rs` | Candle's byte range and checksum, and the bank, calibration and line trays |
| `src/inspect.rs` | Finding candles, the tiles, the known-good bank, the thresholds, the inspector as a `Detector` and the defect region |
| `src/line.rs` | The tray feed as a `FrameSource`, the pipeline and the line's decisions |
| `src/agent.rs` | The harness agent with the vision pack, its three tools and the kernel |
| `src/tape.rs` | Claude's answers recorded live and replayed |
| `src/terminal.rs` | Each second look as it happens, and parts decided by `--cards` |
| `src/serve.rs` | The line page, its routes and the web home behind it |
