# Desk Watch — an agent that watches your desk through a camera

A real agent on the desk camera of your own computer. RF-DETR Nano and a tracker run on every
sampled frame on this computer; watch conditions the agent sets up wake it only when something
it watches for appears, through the Agent Kernel. Woken, the agent loads the kept frame through
the harness vision pack, asks Claude about it through the egress gate — every person covered and
personal text blanked before anything leaves — writes one line to a desk log, and raises a card
held for you, with the frame as evidence, when something needs you.

| ADK-Rust piece | What it does here |
|----------------|-------------------|
| `adk-vision` `DeviceSource` | Captures the camera (AVFoundation, v4l2 or DirectShow) through a user-installed `ffmpeg` |
| `StreamPipeline` | Sampler, frame governor (daily frame quota), privacy hook, RF-DETR Nano, ByteTrack |
| `FrameSources` + `vision_watch` | The agent lists the camera and starts or stops watch conditions (labels, zone, frames in a row, cooldown) |
| `StreamWatch` + `FrameKeeper` | A passing condition keeps its frame, in memory, as the wake payload |
| `adk-kernel` `VisionWakeSource` + `Trigger` | Each watch event starts a kernel process in the desk's session |
| `adk-harness` vision pack | `vision_describe` and `vision_detect` load the kept frame by digest; the egress gate redacts and records what leaves |
| Governed tools | `raise_card` and `vision_watch` start/stop are held for you; `desk_log` is an edit in the desk folder |
| `adk-surface` `PipelineCameras` | The web home's live camera page over the same pipeline: overlays, tracks, zone, watches, governor, switch, raise a card |

```text
camera ──ffmpeg──▶ sampler ─▶ governor ─▶ RF-DETR ─▶ ByteTrack ─▶ watch conditions
                                (frames/day)  (on device)          │ passes 3 frames in a row
                                                                    ▼
                                         frame kept in memory ─▶ VisionWakeSource ─▶ kernel process
                                                                                      │
         Claude (harness, Auto) ◀── wake payload: digest, track, boxes ◀──────────────┘
           ├─ vision_describe ──▶ egress gate: people covered, personal text blanked ──▶ Claude Haiku
           ├─ desk_log ─────────▶ desk/desk-log.md
           └─ raise_card ───────▶ held for you, the frame as evidence
```

## Requirements

| What | Why |
|------|-----|
| Rust 1.95 or later | ADK-Rust 3.0 |
| `ffmpeg` on the `PATH` (`brew install ffmpeg`, `sudo apt install ffmpeg`, `winget install ffmpeg`) | Camera capture; nothing is bundled or downloaded. `ADK_VISION_FFMPEG` names another binary |
| A camera | Any UVC webcam; `--device testsrc` uses ffmpeg's test pattern instead |
| `ANTHROPIC_API_KEY` | The agent (`claude-sonnet-5-5`) and the describer (`claude-haiku-5-5`) |

The build downloads ONNX Runtime. The first run fetches RF-DETR Nano (108 MB) into the Hugging
Face cache, and the first describe fetches PP-OCRv6 small (30 MB) for the personal-text
redactor; every file is checked against adk-vision's built-in manifest before it loads.
`HF_HUB_CACHE` moves the cache, and `HF_HUB_OFFLINE=1` keeps a warm cache from opening a socket.

```bash
cd examples/vision-live-desk
cp .env.example .env          # ANTHROPIC_API_KEY
cargo run -- sources          # the cameras ffmpeg finds
```

```text
  0  camera  Huddly GO
  1  camera  OBS Virtual Camera
  2  screen  Capture screen 0
```

> **Note:** on macOS the first capture asks for camera permission for the app that runs the
> process, usually your terminal. Until it is allowed in System Settings → Privacy & Security →
> Camera, the camera refuses to open and the error says so; run the command again once it is
> allowed.

## Run it headless

```bash
cargo run -- watch --device 0
```

The agent reads what you ask (`--ask`, below the default) and sets up the watches itself with
`vision_watch`. Starting or stopping a watch is held for you on the terminal; so is every card.
Then it sleeps: no model call is made until a watch passes.

```bash
cargo run -- watch --device 0 \
  --ask "Tell me when a phone or a cup is left on the desk while I'm away" \
  --fps 2 --daily-frames 50000 --daily-usd 1.00
```

A run on a desk camera, digests shortened:

```text
[you] Watch my desk. Tell me when a person sits at the desk, and when a cell phone or a cup appears on it.
[call] vision_watch {"action":"list"}
[call] vision_watch {"action":"start","labels":["person"],"minFrames":3,"cooldownSeconds":60,"name":"person-at-desk","zone":[[0,120],[640,120],[640,360],[0,360]],"zoneAnchor":"bottomCenter"}
┌ card held for you: vision_watch … └ approved
[agent] I'm now watching your desk with two conditions …
[watch] person-at-desk fired: person 0.95, track 0, frame 3 (00:02:05)
[wake] [kernel wake "queue:desk:8df5…:0:person-at-desk:detected"] …
[call] vision_describe {"backend":"claude","image":"sha256:9407…","question":"What is the detected person doing, and what objects are on the desk around them? …"}
[answer] vision_describe {"calls":[{"backend":"claude","egress":["sha256:cbf3…"],"remote":true}], …}
[log] - 2026-11-23 00:02:32 person-at-desk fired: a person (score 0.95) at the desk across 3 frames …
┌ card held for you: raise_card
│ title: A person is at the desk
│ evidence: sha256:9407… (640 × 360, artifact vision-9407….png v1, in memory only)
└ declined
[agent] A person was detected at the desk … You declined the card, so the frame wasn't kept.

summary
  frames metered        437
  watches registered    2
  watch events          1
  frames kept (memory)  1
  egress records        1
```

| Flag | Default | Meaning |
|------|---------|---------|
| `--device` | `0` | A camera index or name from `sources`, or `testsrc` |
| `--video-size` | `1280x720` | The capture size; one the camera offers |
| `--fps` | `2` | Frames the detector looks at each second; frames are scaled to 640 px on the long edge |
| `--daily-frames` | `50000` | The frame governor's daily quota: it lowers the rate, then pauses the camera until 00:00 UTC |
| `--threshold` | `0.45` | RF-DETR's score threshold |
| `--zone` | lower two thirds | The desk's zone in sampled-frame pixels, `x0,y0,x1,y1` |
| `--model` | `claude-sonnet-5-5` | The agent |
| `--describer-model` | `claude-haiku-5-5` | What `vision_describe` asks |
| `--egress` | `allow` | `allow` (after covering people and personal text), `ask` (held for you with the exact image), or `deny` |
| `--daily-usd` | `2.00` | The kernel's daily spend cap on model calls |
| `--cards` | `ask` | A held `raise_card`: `ask` on the terminal, `hold`, `approve` or `deny` |
| `--watches` | as `--cards` | A held `vision_watch` start or stop |
| `--minutes` | none | Stops by itself; Ctrl-C stops the camera, then the kernel, any time |
| `--dir` | `desk` | The desk log and approved snapshots |

## Run it in the browser

```bash
cargo run -- serve --device 0 --port 8815
```

Open `http://127.0.0.1:8815/ui/cameras/desk`. The page shows the live frame with RF-DETR's boxes,
the anonymous tracks, the desk zone and its three watches (a person, a phone, a cup), the frame
governor, and the agent's line ("Asleep until someone at the desk"). Everyone the detector calls a
person is covered before a frame reaches the page.

| Act | What happens |
|-----|--------------|
| **Off** | The gate drops every frame, and this example also closes ffmpeg so the camera itself is released; **Camera on** opens it again |
| **Edit the zone** | Moves the desk zone; the watches use it from the next frame |
| **Raise a card from this frame** | Keeps that frame as evidence and sends it to the agent, which describes it once, logs a line and answers in its thread |
| A watch passes | The same kernel process wakes; a `raise_card` it makes waits under **Decisions**, beside the camera |
| **Stop the stream** / Ctrl-C | The pipeline stops and ffmpeg exits |

The agent's thread is at `http://127.0.0.1:8815/ui/`. `--device testsrc` serves the test pattern,
for trying the page without a camera.

The web home sends no browser camera here: its own camera reaches the server only through the
voice screen, which needs a realtime model, so this page reads the same USB device as the
headless run.

## What leaves the machine

| Data | Where it goes |
|------|---------------|
| Camera frames | Nowhere. Frames stay in memory; a wake's frame is kept in an in-memory artifact store and is gone when the process exits |
| A `vision_describe` frame | `api.anthropic.com`, after the egress gate fills every detected person's box (RF-DETR) and personal text (PP-OCRv6) on this computer, under the `--egress` rule; each release is an egress record with its digest |
| Wake payloads, tool answers, the desk log | The agent model, as text: digests, boxes, labels, what the describer said |
| An approved card's frame | `desk/snapshots/desk-<time>.png` with a `.json` caption, people covered again before it is written. `vision-memory` indexes this folder |

When a person sits close to the camera, their box covers most of the frame, so the describer sees
little but the desk's edges: covering the whole box of every detected person is the interim face
policy of adk-vision (spec 3.7 Q3).

## Cost

| Item | Price |
|------|-------|
| `claude-sonnet-5-5` (agent) | $2 / $10 per million input / output tokens |
| `claude-haiku-5-5` (describer) | $0.10 / $0.50 per million tokens; a 640 × 360 frame is about 300 image tokens |
| RF-DETR, ByteTrack, the redactors | Free: on this computer |

Measured on the runs behind this README: one wake — the describe, the log line and the answer —
cost $0.03 to $0.04, and nothing is spent while the agent sleeps. The web home's top bar shows the
day's spend against `--daily-usd`; cooldowns (`cooldownSeconds`) and one wake per track keep a
busy desk cheap.

## Files

| File | Holds |
|------|-------|
| `src/main.rs` | The commands, the headless run, the frame source registration |
| `src/desk.rs` | RF-DETR, the egress rules, the harness with the vision pack, the kernel and its trigger, `desk_log` and `raise_card` |
| `src/terminal.rs` | The agent's turns as they happen, and held calls decided from the keyboard |
| `src/serve.rs` | The web home: `PipelineCameras` over the camera, released while it is off |
