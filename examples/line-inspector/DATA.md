# Data and licences

Nothing large is committed. `cargo run --release -- fetch` downloads only the candle class of
VisA — the archive's first 129 MB, as one byte range — and refuses it unless its size and
SHA-256 match.

## The candles

| Field | Value |
|-------|-------|
| Dataset | VisA, the Visual Anomaly dataset (Zou et al., "SPot-the-Difference Self-Supervised Pre-training for Anomaly Detection and Segmentation", ECCV 2022) |
| Source | `https://amazon-visual-anomaly.s3.us-west-2.amazonaws.com/VisA_20220922.tar`, linked from <https://github.com/amazon-science/spot-diff> |
| Range | Bytes 0–129,238,015: every entry under `candle/`, up to the first entry of the next class |
| SHA-256 of the range | `57d2c00c6ff1255de2372d373edcd066b7802df117e131b48af0e393fbdfae15` |
| Licence | Creative Commons Attribution 4.0 International (CC BY 4.0) for the data, as the upstream README states; the code there is Apache-2.0. Anomalib's documentation lists VisA as CC BY-NC-SA; the dataset's own statement governs here |
| Attribution | "VisA" by Yang Zou, Jongheon Jeong, Latha Pemula, Dongqing Zhang, Onkar Dabeer (Amazon), CC BY 4.0 |
| Content | 1,000 photographs of good trays of four tea lights and 100 with a defect, each with a pixel mask and VisA's name for the defect |

## How the app uses it

| Trays | Use |
|-------|-----|
| Good 0000–0039 | The bank: every candle's nine tiles embedded with SigLIP 2, the known-good reference |
| Good 0100–0129 | The calibration: their candles' scores set the pass threshold (97th percentile) and the reject threshold (the highest) |
| Good 0900–0939 and defective 000–039 | The line: eighty trays interleaved by a fixed shuffle |

`data/bank.json` caches the bank and the calibration's scores after the first run.

## Models

| Model | Where it runs | Licence | How it arrives |
|-------|---------------|---------|----------------|
| SigLIP 2 base, quantized (`siglip2-base-patch16-256-quantized`) | This computer | Apache-2.0 | adk-vision's built-in manifest, fetched by revision and SHA-256 checked |
| SAM 2.1 Hiera Tiny, q4 (`sam2.1-hiera-tiny-q4`) | This computer | Apache-2.0 | The same |
| `claude-sonnet-5-5`, `claude-haiku-5-5` | Anthropic's API | Anthropic's commercial terms | Called with your `ANTHROPIC_API_KEY` in a `--live` run |

## The recording

`recordings/tape.json` holds what Claude answered in the live run behind the README's
results: the agent's turns and the two models' judgements, with their token counts. It holds
no image. A run without `--live` replays it.
