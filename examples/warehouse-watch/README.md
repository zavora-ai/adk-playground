# Warehouse Watch — near-misses on the warehouse floor, put to the shift supervisor

A shift supervisor's safety agent over four ceiling cameras of a warehouse. RF-DETR finds
people in every sampled frame on this computer and ByteTrack follows them. Claude spots
forklifts and pallet trucks once a second. Each camera's calibration places both on the floor
in metres. When a moving vehicle and a person come within 1.5 m, the Agent Kernel starts the
agent in that incident's own session. The agent reads the floor measurement, has Claude review
the four-second clip through `vision_video`, cropped to the two of them, and then either puts a
near-miss on the supervisor's desk for sign-off, with the frame as evidence, or logs why it is
not one. Nothing is spent between incidents.

The warehouse is NVIDIA's PhysicalAI-SmartSpaces `Warehouse_000`: synthetic, with no real
people, and with every person, forklift and pallet truck tracked in 3D. The run measures what
the app found against that ground truth.

| ADK-Rust piece | What it does here |
|----------------|-------------------|
| `adk-vision` `FileSource` + `StreamPipeline` | Plays each camera's video through `ffmpeg`, sampled at 5 frames a second, scaled to 1280 × 720 |
| RF-DETR Nano + `ByteTracker` | People found and followed on this computer, every sampled frame |
| `VisionRuntime::describe` with regions, behind the `EgressGate` | Claude Haiku boxes the forklifts and pallet trucks in one frame a second; every frame it is sent is a recorded release |
| The app's floor watch | Each box's foot point mapped to floor metres through the camera's homography; vehicle speed over two seconds; a person within 1.5 m of a moving vehicle is an incident, with a frame every 0.6 s over four seconds kept as artifacts in its session |
| `adk-kernel` | One `Triggered` process per incident, under a daily spending cap |
| `adk-harness` vision pack | `vision_video` (Claude Sonnet over the clip, `region` cropped to the person and the vehicle) and `vision_annotate`, on the governed path |
| Governed tools | `incident` reads the measurement; `raise_incident` is held for the supervisor with the frame as evidence; `shift_log` writes the shift log |
| `adk-surface` web API | The Warehouse Watch page reads each incident's thread and its held near-miss, and answers them with the surface's `decide` and `send` commands; the web home at `/ui/` is the developer view |

```text
4 cameras ──ffmpeg──▶ 5 fps ─▶ RF-DETR (people) ─▶ ByteTrack ─▶ floor metres ─┐
                         └──▶ 1 fps ─▶ egress gate ─▶ Claude Haiku (vehicles) ─▶ floor metres ─┤
                                                                                              ▼
                     person within 1.5 m of a vehicle moving over 0.5 m/s ─▶ incident: measurement + 4 s clip
                                                                                              │
          kernel process in the incident's session ◀──────────────────────────────────────────┘
            ├─ incident ─────────▶ gap, speed, floor positions, clip, clip region
            ├─ vision_video ─────▶ egress gate ─▶ Claude Sonnet: the clip, cropped to the two of them
            └─ raise_incident ───▶ held on the supervisor's desk, the frame as evidence   (or shift_log)
```

## Requirements

| What | Why |
|------|-----|
| Rust 1.95 or later | ADK-Rust 3.0 |
| `ffmpeg` on the `PATH` (`brew install ffmpeg`, `sudo apt install ffmpeg`, `winget install ffmpeg`) | Decoding the videos; nothing is bundled. `ADK_VISION_FFMPEG` names another binary |
| 1.3 GB of disk | The four cameras, their calibration and the ground truth |
| `ANTHROPIC_API_KEY`, for `--live` only | The spotter and the clip review (`claude-haiku-5-5`, `claude-sonnet-5-5`) and the agent (`claude-sonnet-5-5`) |

The build downloads ONNX Runtime. The first run fetches RF-DETR Nano (108 MB) into the Hugging
Face cache, checked against adk-vision's built-in manifest before it loads.

```bash
cd examples/warehouse-watch
cargo run --release -- fetch
```

```text
fetching nvidia/PhysicalAI-SmartSpaces@d38193be60 MTMC_Tracking_2026/train/Warehouse_000
  fetching Camera_0001.mp4         242275945 bytes from nvidia/PhysicalAI-SmartSpaces@d38193be60
  …
  cutting ground_truth.json to 5 frames a second for the four cameras…
  wrote    truth.json: 1500 samples, 76 objects
```

[DATA.md](DATA.md) lists every file with its licence and SHA-256.

## Run it headless

```bash
cargo run --release -- run            # replays the recorded answers: no key, no spend
cargo run --release -- run --live     # Claude live; its answers are recorded again
```

The agent sleeps until the floor watch finds a close approach, then reviews it in the
incident's own session. A replay of the recorded shift, identifiers shortened:

```text
[watch] c01-022799 Camera_0001: person 12 and a forklift 0.5 m apart at 0:22.8 (0.9 m/s)
[c01-022799] call incident {"id":"c01-022799"}
[c01-022799] call vision_video {"frames":["c01-022799-0.png",…,"c01-022799-6.png"],"question":"Is a forklift or pallet truck near the person? …","region":[…],"timesMs":[…]}
[c01-022799] call raise_incident {"detail":"In the open aisle on Camera_0001 at 0:22.8, a forklift moving at 0.9 m/s came within 0.5 m of a person, with no racking or wall between them. …"}
┌ [c01-022799] held for the supervisor: Forklift passed 0.5 m from a person on Camera_0001
└ held: it waits on the supervisor's desk (`serve` shows it)
[watch] c01-026000 Camera_0001: person 12 and a forklift 0.2 m apart at 0:26.0 (1.0 m/s)
[c01-026000] call shift_log {"line":"c01-026000 Cam1 0:26: not raised. Person walked to a parked pallet truck, boarded it and rode off as the driver. …"}
…
results (5:00.0 of video, 4 cameras, in 289s)
  ground-truth near-misses       43 (a moving vehicle within 1.0 m of a person, both in view)
  close approaches on the floor  31 → found 8 of 43 near-misses (recall 0.19), 11 on a near-miss (precision 0.35)
    of the 20 a camera shows clearly    found 8 (recall 0.40)
  raised by the agent            18 → found 8 of 43 (recall 0.19), 10 on a near-miss (precision 0.56)
    of the 20 a camera shows clearly    found 8 (recall 0.40)
  dismissed into the shift log   13
  agent  claude-sonnet-5-5   112 calls    884739 in   27715 out  $0.7604
  spot   claude-haiku-5-5   1200 calls   2172000 in  380349 out  $0.4616
  video  claude-sonnet-5-5    31 calls     74440 in   13839 out  $0.3244
  total model spend              $1.5464 (the recorded live run's; this replay spent nothing)
```

### What the numbers say

| Stage | Reported | Precision | Recall: all 43 | Recall: the 20 a camera shows clearly |
|-------|---------:|----------:|---------------:|--------------------------------------:|
| Floor watch (RF-DETR people, Claude vehicles, calibration) | 31 | 0.35 | 0.19 | 0.40 |
| Raised by the agent after reviewing the clip | 18 | 0.56 | 0.19 | 0.40 |

- **The review keeps what the watch found and drops most of the rest.** Every near-miss the
  watch caught was still raised after the agent's review. Of the 20 approaches on no
  ground-truth near-miss, it dismissed 12 into the shift log, each with its reason — most often a
  vehicle parked in every frame while a person walked past — and raised 8. Of the 11 on a
  near-miss it raised 10; the one it dismissed shows a person stepping onto a pallet truck as its
  driver, and that near-miss was raised from another approach.
- **Most misses are vehicles hidden behind racking.** The ground truth's visible boxes include
  vehicles seen through or behind shelving, and many of its near-misses put a person inside a
  vehicle's footprint, where the camera shows one object behind the other. Claude boxes the
  vehicles a person can see; with the forklift clear in view, the watch finds two in five.
- **Speed comes from the floor, not the clip.** A forklift that ground truth has at 0.8–1.0 m/s
  read as "standing" to Claude Haiku over a short, uncropped clip. The agent now reviews with
  Claude Sonnet over a crop of the two of them, and the prompt makes the calibrated
  measurement the authority on distance and speed.
- **A near-miss is defined before the run.** Ground truth: a forklift or pallet truck moving
  over 0.3 m/s within 1.0 m of a person, both visible in one of the four cameras; a camera
  shows a pair clearly when the vehicle's visible box covers 3,000 px and the person's is
  80 px tall at some moment of the event. A report matches an event within 3 s and 3 m.

The live run's cost — $1.55 for five minutes of four cameras — is mostly the agent's reviews
and the spotter: $0.46 for 1,200 Haiku calls. A detector trained on the site's own vehicles
and loaded through adk-vision's user manifests would spot them on the device for nothing.

| Flag | Default | Meaning |
|------|---------|---------|
| `--live` | off | Calls Claude and records its answers to `recordings/tape.json`; without it the recording replays |
| `--seconds` | the whole five minutes | Watches only the start of the shift |
| `--cards` | `hold` | A near-miss held for the supervisor: `hold` leaves it on the desk, `approve` signs it off, `deny` declines it |
| `--agent-model` | `claude-sonnet-5-5` | The agent |
| `--spot-model` | `claude-haiku-5-5` | The vehicle spotter |
| `--video-model` | `claude-sonnet-5-5` | What `vision_video` asks |
| `--daily-usd` | `5.00` | The kernel's daily spending cap |
| `--out` | `out` | The shift log, signed-off incident reports and `results.json` |
| `--port` (`serve`) | `8816` | Where the page listens, on `127.0.0.1` |

Every mode reads the cameras losslessly: `run` plays them as fast as the machine can, and
`serve` plays them at the video's own speed by holding each frame until its time. A busy
machine plays the shift late but never skips a frame, so a replay meets the approaches the
recording met, and a live `serve`, reading files rather than cameras, records a shift that
replays the same way. A replay reproduces the recording on a machine whose ffmpeg and ONNX
Runtime decode and detect the same frames the same way; a machine that sees an approach the
recording does not reports it as an incident with no recorded review.

## Run it in the browser

```bash
cargo run --release -- serve            # replayed: no key, no spend
cargo run --release -- serve --live     # live
```

Open `http://127.0.0.1:8816/warehouse`. The page is the whole app for the shift supervisor:

| Part | What it shows and does |
|------|------------------------|
| Header | **Replay** — a recorded shift played back with no key and no spend, its spend read as the recorded run's — or **Live**, with the spend so far against `--daily-usd`; the shift's progress (`1:20 of 5:00`) and **Start the shift**. The cameras wait for it; when the video ends the button reads **Shift finished** |
| Cameras | Each camera's first frame until the shift starts, then the video at its own speed, the people found on this computer outlined in green and the vehicles Claude spotted in orange |
| Close approaches | Every approach as it is measured, with its status: reviewing, waiting for your sign-off, signed off, declined, or logged as not a near-miss |
| The incident | Opens from a row: the closest frame with the person and the vehicle boxed, the clip's other frames, the measured gap and speed, and the agent's review |
| Your decision | A near-miss the agent raised, with its report and the frame. **Approve** signs it off: the agent writes `out/incidents/<id>.json` and a line in the shift log. **Decline** takes an optional reason the agent reads. The card then reads "You approved at 08:15" |
| Conversation and ask box | The incident's thread — your questions, the agent's replies, its tool steps as one line per turn — and **Ask about c01-022799…**. A turn that fails shows its error in its place |

In a replay the ask box is off: questions need a live run. A turn the recording does not hold,
such as the agent's reply after a decision, answers "This is a replay, and this answer was not
recorded. Run with --live for a real answer." instead of failing; the headless `run` keeps that
miss an error, since it checks the replay against the recording. **Developer view** in the
footer opens the ADK-Rust web home at `/ui/`.

## What leaves the machine

| Data | Where it goes |
|------|---------------|
| One frame a second per camera, 1280 × 720 | `api.anthropic.com` (the spotter), through the egress gate, each release recorded |
| An incident's clip frames, cropped to the person and the vehicle | `api.anthropic.com` (`vision_video`), through the same gate |
| Measurements, ids and the agent's notes | The agent model, as text |
| Everything else | Nowhere. Frames stay in memory; an incident's clip is kept in the in-memory artifact store until the process exits |

Every restriction is an opt-in, as everywhere in adk-vision, and this example sets none: the
dataset has no real people. The agent's `vision_video` calls follow `out/.adk/vision.toml` when it
exists, through the harness vision pack: `[egress] default = "ask"` holds each clip for a person
with the exact frames it would send, and `deny` keeps clips on this machine. The spotter's gate has
no rules of its own; a site that wants no frame to leave runs a detector trained on its own
vehicles on the device instead, through adk-vision's user manifests.

## Files

| File | Holds |
|------|-------|
| `src/main.rs` | The commands, the headless run and its report |
| `src/data.rs` | The pinned dataset, the fetch with size and SHA-256 checks, the calibration and the cut-down ground truth |
| `src/watch.rs` | The cameras, read losslessly: people, the vehicle spotter, floor metres, close approaches and their clips |
| `src/agent.rs` | The harness agent with the vision pack, its three tools, and the kernel |
| `src/eval.rs` | Ground-truth near-misses and the scoring |
| `src/tape.rs` | Claude's answers recorded live and replayed |
| `src/terminal.rs` | Each review as it happens, and held cards decided by `--cards` |
| `src/serve.rs` | The Warehouse Watch page: the shift, the cameras, each incident with its decision and conversation |
