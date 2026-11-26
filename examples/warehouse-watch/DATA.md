# Data and licences

Nothing large is committed. `cargo run --release -- fetch` downloads each file below from the
pinned revision and refuses any file whose size or SHA-256 differs.

## The warehouse

| Field | Value |
|-------|-------|
| Dataset | NVIDIA PhysicalAI-SmartSpaces, `MTMC_Tracking_2026/train/Warehouse_000` |
| Source | <https://huggingface.co/datasets/nvidia/PhysicalAI-SmartSpaces> |
| Revision | `d38193be60906679124732a4be9a6516821e2bed` |
| Licence | Creative Commons Attribution 4.0 International (CC BY 4.0), as the dataset card states |
| Attribution | "PhysicalAI-SmartSpaces" by NVIDIA, CC BY 4.0. Frames shown on the Warehouse Watch page and kept as incident evidence are unmodified frames of this dataset, scaled; boxes drawn on them are this app's |
| Content | Synthetic: rendered in NVIDIA Omniverse, with no real people |

| File | Bytes | SHA-256 |
|------|------:|---------|
| `videos/Camera_0001.mp4` | 242,275,945 | `aebf51f04434f0cccfb27d9e0385a5bde5d91a15307a91aa17f112088c6e7256` |
| `videos/Camera_0003.mp4` | 221,996,855 | `7668fc49c5903174626894ceb7cc2fd30a69c362a9a2734d7e75a3f2b4eb15db` |
| `videos/Camera_0005.mp4` | 188,989,348 | `85e359566e0bd4bdd0db6eaf28ac171b21e4af80f64bc461e703c7935b65e45b` |
| `videos/Camera_0015.mp4` | 212,738,760 | `0092b24cfc01e716784f9e302ad5b08f9dd9509a8a60414e67f2d9d7fcb2d2ed` |
| `calibration.json` | 44,111 | `93091b6e4a47600075a8331135982d23e1f44d5a9abb5fb2959cbe44216257c5` |
| `ground_truth.json` | 281,636,586 | `0c160199a14f04bcb95e2ece7c9c3be37f1ffa29a4b1fd1a390c607d21c28109` |
| `map.png` | 1,123,636 | `0bb7fd5915654b6fa4a1e984d9ed4d83eec41a7ebab8a045badf16a40348890a` |

`fetch` also writes `data/truth.json`, a derivative of `ground_truth.json` cut to five samples
a second (each object's floor position, footprint and visible boxes in the four cameras). It
stays on your machine.

## Models

| Model | Where it runs | Licence | How it arrives |
|-------|---------------|---------|----------------|
| RF-DETR Nano (`rf-detr-n`, `onnx-community/rfdetr_nano-ONNX`) | This computer | Apache-2.0 | adk-vision's built-in manifest, fetched by revision and SHA-256 checked before ONNX Runtime loads it |
| `claude-haiku-5-5`, `claude-sonnet-5-5` | Anthropic's API | Anthropic's commercial terms | Called with your `ANTHROPIC_API_KEY` in a `--live` run |

## The recording

`recordings/tape.json` holds what Claude answered in the live run behind the README's
results: the spotter's boxes per camera and second, and the agent's and the clip review's
responses, with their token counts. It holds no image. A run without `--live` replays it.
