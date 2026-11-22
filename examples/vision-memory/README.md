# Recall — an agent with visual memory

An agent that remembers what it has seen. Desk snapshots kept by
[`vision-live-desk`](../vision-live-desk/), your photos and your scanned documents are embedded
with SigLIP 2 and labelled by RF-DETR on this computer, stored in a vector store that survives a
restart, and found again by a question ("when did I last see my red mug?") or by an example image
("find receipts like this one"). Every answer cites the image it rests on, and "forget everything
from the desk camera" erases a source after you allow it.

| ADK-Rust piece | What it does here |
|----------------|-------------------|
| `adk-vision` `VisualMemory` | One chunk per image in an `adk-rag` store: the image's SigLIP 2 vector, its caption, its artifact, source, capture time and labels; one embedding space per collection, checked on open |
| `SigLip2` (`siglip2-base-patch16-256-quantized`) | Texts and images in one space, on this computer |
| `RfDetr` | Labels each image on the device; the labels join its caption |
| `MediaStore` over `FileArtifactService` | The images, stored by digest under `memory-store/images` |
| `erase_source` | Deletes a source's vectors and the images no other source uses, and writes `vision.memory.erased` |
| `adk-agent` `LlmAgent` | Claude drives the tools through the governed path; `forget_source` needs confirmation |

| Tool | Effect | Does |
|------|--------|------|
| `memory_sources` | read-only | What the memory holds: each source, its kind, how many images, the time they span |
| `index_folder` | idempotent | Indexes a folder under a source name; images already indexed are skipped |
| `recall` | read-only | Searches by a short visual description; with `contrast`, each hit carries its margin over the closest alternative |
| `recall_like` | read-only | Searches by an example image: a file in a permitted folder, or an indexed image's digest |
| `forget_source` | held for you | Erases a source |

## Requirements

| What | Why |
|------|-----|
| Rust 1.95 or later | ADK-Rust 3.0 |
| `ANTHROPIC_API_KEY` | The agent, `claude-sonnet-5-5` |

The build downloads ONNX Runtime. The first run fetches SigLIP 2 base quantized (381 MB) and
RF-DETR Nano (108 MB) into the Hugging Face cache, each file checked against adk-vision's built-in
manifest. `HF_HUB_CACHE` moves the cache.

```bash
cd examples/vision-memory
cp .env.example .env          # ANTHROPIC_API_KEY
```

## Try it on the generated week

```bash
cargo run -- demo-corpus
```

`demo-corpus` draws twelve desk snapshots from Monday 16 to Saturday 21 November 2026 (a laptop,
papers, a book, a plant, a phone, a red mug and a blue mug in different places) in the layout
`vision-live-desk` keeps approved cards in, nine scanned pages from adk-vision's generated week
(delivery notes, inspection reports and invoices, all data invented), and two query images that
are in neither folder. Nothing is downloaded; nothing here is a photo of anything real. Captions
are neutral ("Desk camera snapshot 6"), so every answer below comes from the pixels.

```bash
cargo run -- ask --confirm yes \
  "Index my desk snapshots in demo/snapshots (source desk-camera, camera frames) and my scanned paperwork in demo/documents (source documents, pages)." \
  "When did I last see my red mug on the desk?" \
  "Find receipts like this one: demo/queries/receipt-example.png" \
  "Forget everything from the desk camera." \
  "What do you still remember?"
```

A run, paths and digests shortened:

```text
[call] index_folder {"folder":"demo/snapshots","kind":"frame","source":"desk-camera"}
[call] index_folder {"folder":"demo/documents","kind":"page","source":"documents"}
[answer] index_folder {"indexed":12,"alreadyIndexed":0, …}

[you] When did I last see my red mug on the desk?
[call] recall {"text":"a red coffee mug on a desk","contrast":"a blue coffee mug on a desk","topK":10}
[recall] Your red mug was most likely last on the desk on Friday 2026-11-20 at 16:40 +03:00.
- That snapshot is desk-20261120-164000.png from desk-camera, and a cup was detected in it
  (artifact://vision-memory/me/desk-camera/vision-741b….png#1). Its margin over "blue mug" is +0.036.
- Later snapshots, such as desk-20261121-111500.png from Sat 2026-11-21 at 11:15, also show a cup.
  Those lean blue (margin −0.04), so I don't count them as the red mug.

[you] Find receipts like this one: demo/queries/receipt-example.png
[call] recall_like {"example":"demo/queries/receipt-example.png","topK":8}
[recall] Three scanned pages look like your example, at 0.979 to 0.984; the next page drops to 0.930.
- scan-07.png (documents, Wed 2026-11-18 10:20), score 0.984: artifact://vision-memory/me/documents/vision-1b61….png#1
- scan-05.png (documents, Tue 2026-11-17 14:16), score 0.981: artifact://…/vision-4be8….png#1
- scan-03.png (documents, Mon 2026-11-16 18:16), score 0.979: artifact://…/vision-359c….png#1

[you] Forget everything from the desk camera.
┌ vision-memory wants to run forget_source {"source":"desk-camera"}
└ allowed
[answer] forget_source {"chunksDeleted":12,"imagesDeleted":12,"imagesKept":0,"source":"desk-camera"}
```

The three pages are exactly the corpus's three invoices, and Friday 16:40 is the red mug's last
appearance. The transcript joins two runs: the red-mug question and the erasure ran in a second
process, on the store the first one wrote, with nothing indexed again.

`--confirm ask` (the default) asks on the terminal before a source is erased; `--confirm no`
declines. `chat` holds a conversation instead of a list of questions.

## Your own images

```bash
cargo run -- ask --allow ~/Pictures/receipts \
  "Index ~/Pictures/receipts as receipts, they are photos" \
  "Find receipts like this one: ~/Pictures/receipts/2026-11-03.jpg"
```

The agent indexes and reads only folders given with `--allow` (by default `demo` and
`../vision-live-desk/desk/snapshots`, where the live desk keeps the frames you approve). A `.json`
beside an image may give its `caption` and `capturedAt`; without it, the file's modification time is
the capture time.

| Command | Does |
|---------|------|
| `demo-corpus [--out demo]` | Writes the generated corpus |
| `index <folder> --source <name> --kind frame\|page\|image` | Indexes a folder directly, without the agent |
| `search "<text>"` / `search --like <image>` | Prints the ranked hits directly, without the agent |
| `ask "<question>"…` | Asks the agent, in one conversation |
| `chat` | Talks with the agent on the terminal |

Every command takes `--store` (default `memory-store`), `--allow`, `--model` and `--confirm`.

## What leaves the machine

| Data | Where it goes |
|------|---------------|
| Images | Nowhere: embedded and labelled on this computer, stored under `memory-store/` |
| Captions, labels, scores, capture times, file names, artifact URIs | The agent model, as text |

## How well it finds things

SigLIP 2's text-to-image similarities are small and only comparable within one search. On the
generated week, "a red coffee mug on a desk" ranks the four red-mug snapshots first, but the blue
mugs follow closely (0.131 for the last red mug, 0.118 for the first blue one), so a plain
"latest hit" would be wrong. `recall` with a `contrast` ("a blue coffee mug on a desk") gives each
hit its margin: the four red-mug snapshots have margins of +0.033 to +0.065, the three blue-mug
snapshots −0.041 to −0.049, and the snapshots with no mug score 0.064 or less whatever their margin.
Searches by example image separate far more: the three invoices score 0.979 to 0.984 against the
example invoice, the other pages 0.93 at most. Indexing the twelve snapshots took 3.4 s on a laptop
CPU.

## The store

`memory-store/vectors.json` is adk-rag's in-memory vector store with every change written through
to one file, so the example needs no database. `VisualMemory` uses only the `VectorStore` trait: for
more than a few thousand images, replace `FileVectorStore` in `src/memory.rs` with
`adk_rag::LanceDBVectorStore` (adk-rag feature `lancedb`, which needs `protoc`) or `PgVectorStore`.
The collection records SigLIP 2's embedding space and refuses another embedder, so changing models
means a new store.

## Files

| File | Holds |
|------|-------|
| `src/main.rs` | The commands, the agent and its confirmation on the terminal |
| `src/memory.rs` | `Recall`: SigLIP 2, RF-DETR, the visual memory, the manifest from each image to its file |
| `src/tools.rs` | The five tools |
| `src/store.rs` | `FileVectorStore` |
| `src/corpus.rs` | The generated corpus |
