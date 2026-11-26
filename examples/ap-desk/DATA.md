# Data and licences

Nothing large is committed. `cargo run --release -- fetch` downloads FATURA once, refuses it
unless its size and SHA-256 match, and builds the inbox from it.

## The invoices

| Field | Value |
|-------|-------|
| Dataset | FATURA: a multi-layout invoice image dataset for document analysis and understanding |
| Source | Zenodo record 8261508, <https://zenodo.org/records/8261508> |
| File | `FATURA.zip`, 363,504,027 bytes, SHA-256 `c8bf2ee6ba1b7a678954b7b89b31f9387100dc6b9e990c076aff5a232a9f53d7` |
| Licence | Creative Commons Attribution 4.0 International (CC BY 4.0), as the Zenodo record states |
| Attribution | "FATURA" by Mahmoud Limam, Marwa Dhiaf, Yousri Kessentini, CC BY 4.0 |
| Content | 10,000 synthetic invoices in 50 layouts, rendered at 595 × 841 pixels, with invented companies, people and addresses |

## What `fetch` builds from it

| File | What it is |
|------|------------|
| `data/inbox/d01.jpg` … `d44.jpg` | The first four invoices of FATURA's test split for ten layouts, one supplier each: 31 as rendered, 9 as phone photos (tilted up to 1.6°, blurred, noisy, JPEG quality 40), and 4 second copies of earlier invoices, scanned. Every derived image is made from a fixed seed, so every fetch makes the same files |
| `data/books.json` | The ERP the desk matches against — nine approved suppliers, a purchase order for each invoice that prints a PO number (three for less than was invoiced, two with no goods receipt) — and each document's expected header and decision |

The expected header comes from FATURA's annotations: `SELLER_NAME`, `NUMBER`, `DATE`,
`PO_NUMBER` and `TOTAL`, with their labels taken off. The expected decision applies the desk's
policy to the generated ERP. Subtotal, discount and tax are left out: FATURA's generator does
not keep them consistent with the total (9 of the 5,597 invoices that print all four add up).

## Models

| Model | Where it runs | Licence | How it arrives |
|-------|---------------|---------|----------------|
| PP-OCRv6 small (`pp-ocrv6-small-det`, `pp-ocrv6-small-rec`) | This computer | Apache-2.0 | adk-vision's built-in manifest, fetched by revision and SHA-256 checked |
| SigLIP 2 base, quantized (`siglip2-base-patch16-256-quantized`) | This computer | Apache-2.0 | The same |
| `claude-sonnet-5-5` | Anthropic's API | Anthropic's commercial terms | Called with your `ANTHROPIC_API_KEY` in a `--live` run |

## The recording

`recordings/tape.json` holds what Claude answered in the live run behind the README's results:
the agent's turns and the crop readings, with their token counts. It holds no image. A run
without `--live` replays it.
