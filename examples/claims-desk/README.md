# Claims Desk — motor claims assessed from their photos, estimates drafted for the adjuster

A motor claims agent over a day's nineteen claims. Each claim arrives as a first notice of loss
and one or two photos. On this computer RF-DETR finds the vehicle, SAM 2.1 outlines the damaged
parts, and SigLIP 2 remembers every photo it has seen. Claude assesses the damage part by part.
The agent checks that every photo is new and shows the insured vehicle, then drafts the estimate
for the adjuster, priced from an illustrative list — or refers the claim to investigations when
a photo was used before or shows another car.

The photos are real, curated from Wikimedia Commons with each file's licence and author recorded,
and their severity is the category Commons editors filed them in; the run scores the desk against
those categories and the claims' seeded cases.

| ADK-Rust piece | What it does here |
|----------------|-------------------|
| `vision_detect` (RF-DETR Nano) | Finds the vehicle in each photo, on this computer |
| `vision_describe` behind the `EgressGate` | Claude Sonnet answers a schema: the vehicle's make, colour and body style, each damaged part with its side, kind, severity and box, and the overall severity |
| `vision_segment` (SAM 2.1 Tiny) | Outlines the moderate and major parts from Claude's boxes, on this computer |
| `VisualMemory` with SigLIP 2 | `similar_photos`: every earlier claim's photos, so a reused one is found again |
| The app's tools | `claim` reads the first notice of loss; `draft_estimate` and `refer_claim` are held for the adjuster with the photos |
| `adk-kernel` | One `Triggered` process per claim, in the claim's own session, in arrival order |
| `adk-surface` | The decisions and threads the Claims Desk page reads and answers through `/api/surface`; the web home at `/ui/` as the developer view |

```text
CL-0404: first notice of loss + 2 photos ─▶ kernel process in session CL-0404
            ├─ claim ─────────────▶ policy, insured vehicle, date, what happened
            ├─ vision_detect ─────▶ RF-DETR on this computer: the vehicle
            ├─ vision_describe ───▶ egress gate ─▶ Claude: make, colour, damaged parts with boxes, severity
            ├─ vision_segment ────▶ SAM 2.1 on this computer: the parts outlined
            ├─ similar_photos ────▶ SigLIP 2 on this computer: the same photo in an earlier claim?
            └─ draft_estimate  or  refer_claim ─▶ held on the adjuster's desk with the photos
```

## Requirements

| What | Why |
|------|-----|
| Rust 1.95 or later | ADK-Rust 3.0 |
| 100 MB of disk | The twenty photos as Commons holds them, and the scaled copies |
| `ANTHROPIC_API_KEY`, for `--live` only | The agent and the damage assessor (`claude-sonnet-5-5`) |

The build downloads ONNX Runtime; the first run fetches RF-DETR Nano, SAM 2.1 Tiny q4 and
SigLIP 2 base quantized into the Hugging Face cache, each file checked against adk-vision's
built-in manifest.

```bash
cd examples/claims-desk
cargo run --release -- fetch
```

```text
fetching the claim photos from Wikimedia Commons
  fetching Abandoned Suzuki, Osidge, London.jpg (3860860 bytes) — Philafrenzy, CC BY-SA 4.0
  …
  wrote the photos of 19 claims to data/photos
```

Wikimedia slows down clients that do not say who they are: the fetch names this example and its
repository, waits between files and backs off when asked. Set `WIKIMEDIA_USER_AGENT` to add your
own contact, as Wikimedia's policy suggests. [DATA.md](DATA.md) lists every photo with its author,
licence and SHA-1.

## Run it headless

```bash
cargo run --release -- run            # replays the recorded answers: no key, no spend
cargo run --release -- run --live     # Claude live; its answers are recorded again
```

A replay of the recorded day, the four claims that were referred:

```text
┌ [CL-0404] on the adjuster's desk, referral (different_vehicle): Photo 2 shows a red Chevrolet Sonic, not the insured black Civic
│ CL-0404-2.jpg shows a red Chevrolet Sonic sedan, while the insured vehicle and CL-0404-1.jpg are a black Honda Civic. The make and colour differ, so this photo is left out of the assessment.
┌ [CL-0414] on the adjuster's desk, referral (different_vehicle): Photo 2 shows a panel van, not the insured Lupo
│ CL-0414-2.jpg shows a white Citroën C15 panel van with right-side damage and rust. The insured vehicle is a white 2001 Volkswagen Lupo hatchback, and photo 1 shows a hatchback.
┌ [CL-0418] on the adjuster's desk, referral (reused_photo): CL-0418 photo matches earlier claim CL-0403
│ CL-0418-1.jpg scores 0.983 similarity against CL-0403-1.jpg from an earlier claim, above the 0.95 level that marks a reused photo. …
┌ [CL-0419] on the adjuster's desk, referral (reused_photo): CL-0419 photo matches earlier claim CL-0405
│ CL-0419-1.jpg has a similarity of 0.97 to CL-0405-1.jpg from an earlier claim, above the 0.95 threshold for a reused photo. …
…
results (19 claims in 36s)
  outcomes right                    19 of 19
    expected different_vehicle  → different_vehicle 2
    expected estimate           → estimate 15
    expected reused_photo       → reused_photo 2
  severity, against Commons         9 of 15 the same; 13 of 15 within one step
  damaged side, where Commons names it  3 of 3
  agent     claude-sonnet-5-5    75 calls   754532 in  26521 out  $0.6368
  assessor  claude-sonnet-5-5    25 calls    72410 in  10162 out  $0.2826
  total model spend                 $0.9194 (the recorded live run's; this replay spent nothing)
```

### What the numbers say

| Measure | Result |
|---------|--------|
| Claims that came to the right outcome | 19 of 19: 15 estimates, both reused photos referred, both other vehicles referred |
| Reused photos caught | 2 of 2, at 0.97 and 0.98 similarity to the photo of the earlier claim; no false alarm |
| Photos of another vehicle caught | 2 of 2, each named by make, colour and body style; no false alarm |
| Severity against the Commons category | 9 of 15 the same, 13 of 15 within one step |
| Damaged side, where the Commons title names it | 3 of 3 |
| Cost of the live day | $0.92 |

- **The agent rates damage higher than Commons does.** Every claim Commons files as major came out
  major, but so did four of its six moderate ones and both of its minor ones. A crushed wing with
  the wheel pushed in reads as structural damage to the agent; the Commons editors who filed the
  photos called it minor. An insurer would calibrate the scale against its own adjusters'
  past decisions.
- **A reused photo is found by the photo, not by the story.** Both reused photos came with a new
  policy, a new date and a new account of what happened; SigLIP 2 found each against the earlier
  claim's photo on this computer, through a crop and a re-compression.
- **Another vehicle is found by comparing what the photos show.** Both cases name the insured make
  and colour; the agent reads the other car's make, colour and body style from the photo and leaves
  it out of the assessment.

| Flag | Default | Meaning |
|------|---------|---------|
| `--live` | off | Calls Claude and records its answers to `recordings/tape.json`; without it the recording replays |
| `--claims` | all 19 | Works only the first claims |
| `--cards` | `hold` | An estimate or referral held for the adjuster: `hold`, `approve` or `deny` |
| `--agent-model` | `claude-sonnet-5-5` | The agent |
| `--assessor-model` | `claude-sonnet-5-5` | What `vision_describe` asks to assess a photo |
| `--daily-usd` | `5.00` | The kernel's daily spending cap |
| `--out` | `out` | `results.json` |
| `--port` | `8819` | `serve` only: the page's port |

## Run it in the browser

```bash
cargo run --release -- serve                      # replayed: no key, no spend
cargo run --release -- serve --live               # live: answers recorded again
cargo run --release -- serve --live --claims 2    # a short live day
```

Open `http://127.0.0.1:8819/claims`. The page is the whole desk. The day waits until you press
**Start the day**; the desk then works the claims in the order they arrived.

| Part of the page | What it shows and does |
|------------------|------------------------|
| Header | `Replay` or `Live`; claims worked, estimates, referrals and the decisions waiting for you; the spend — the recorded run's in a replay (`$0.92 when it was recorded`), this run's against the daily cap when live (`$0.31 of $5.00`). After the day the button reads **Day finished**: restart the app to run it again |
| Today's claims | Each claim with where it is — not started, assessing, its estimate or referral — and **Your decision**, **Approved** or **Declined** |
| What the desk found | The claim's photos, policy, insured vehicle, date and account of what happened, and the desk's assessment |
| Your decision | An estimate with its lines, hours, amounts and total; or a referral with its reason — a reused photo beside the earlier claim's photo and how alike they score, or the photo of another vehicle beside the claim's other photo. **Approve**, or **Decline** with an optional reason the desk reads. The card then reads "You approved at 08:41" or "You declined at 08:41: …" |
| Conversation | Your questions and the desk's replies. A turn's tool steps fold into one line ("Worked for 7 s · 8 steps"); text the desk wrote between steps is not shown as a reply |
| Ask box | "Ask about CL-0413…" sends a question in the claim's own session. "Claims Desk is answering…" shows while the turn runs; a failed turn shows its error |

In a replay, decisions work and new questions need a live run, so the ask box is disabled. A turn
the recording does not hold — the desk's closing reply after your decision — answers "This is a
replay, and this answer was not recorded. Run with --live for a real answer." The headless `run`
keeps such a miss an error: it is the check that the replay is the recorded day.

**Developer view** in the footer opens ADK-Rust's web home at `/ui/`, with each claim's thread
and decisions as the framework draws them.

## What leaves the machine

| Data | Where it goes |
|------|---------------|
| Each claim photo, scaled to 1568 pixels | `api.anthropic.com`, through the egress gate, each release recorded |
| Detection, outlines and the photo memory | Nowhere: RF-DETR, SAM 2.1 and SigLIP 2 run on this computer |
| The first notice of loss, the assessments | The agent model, as text |

Every restriction is an opt-in, and this example sets none. `out/.adk/vision.toml` with
`redact = ["faces"]` covers any face before a photo leaves;
`[egress] default = "ask"` holds each photo for a person with the exact image it would send.

## Files

| File | Holds |
|------|-------|
| `src/main.rs` | The commands, working the claims, and the report |
| `src/data.rs` | The pinned Commons files, the claims and their first notices of loss, the price list and the fetch |
| `src/desk.rs` | The harness agent with the vision pack, the visual memory, the four tools and the kernel |
| `src/tape.rs` | Claude's answers recorded live and replayed; under `serve`, an agent answer the recording lacks is said in words |
| `src/terminal.rs` | Each claim's turn as it happens, and held calls decided by `--cards` |
| `src/serve.rs` | The Claims Desk page and its state, Start the day, and the web home beside it |
