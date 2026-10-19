# Offline CLIP tile search: feasibility

Researched 2026-10-19. Desk research only. No model was downloaded and no experiment was run (downloading is not pre-approved), so every speed figure below is an estimate from public benchmarks and FLOP counts, not a measurement here.

## Recommendation: do it later (P5 stretch), behind a one-day quality gate

Technically it is feasible and cheap to build. The open risk is retrieval quality on dark 96 px pixel art, which nobody has published numbers for. So:

1. Do not start crate work now. P1 and P3 (pHash, FTS, generated names, colour and texture filters) already cover most "find me X" needs.
2. When P5 starts, spend one day on a Python quality spike: 100 labelled tiles, about 30 queries, precision@10 against the existing `gen_name` FTS baseline. Go only if CLIP clearly beats the baseline on material and mood queries ("mossy", "bloody", "frozen", "ruined brick").
3. Design the hooks now (see "Hooks to reserve now"), since they cost almost nothing.
4. Do not embed the weights in the exe. Use an opt-in install into `capybara\cache\clip\` (details below).

## Chosen stack (if the gate passes)

| Piece | Choice |
|---|---|
| Inference | `ort` 2.0.0-rc.14 (2026-10-12; docs call it production-ready; MIT OR Apache-2.0), feature `load-dynamic`. We ship no ONNX Runtime; the opt-in install puts Microsoft's CPU-only `onnxruntime.dll` (MIT, about 12 MB, x64) next to the models, and we call `ort::init_from(<abs path>)`. |
| Default model | OpenCLIP ViT-B/32 `laion2b_s34b_b79k` (MIT, 512-d, 66.6% ImageNet zero-shot), exported once by us to ONNX and dynamically int8-quantized. Hosted on the project's own release page. |
| Lite model | TinyCLIP ViT-22M/32 Text-10M (Microsoft, MIT, 53.7% ImageNet), int8, about 35-45 MB, roughly 2x faster. Same code path; the model id is stored per vector. |
| Tokenizer | CLIP BPE (49,408 vocab, 77 tokens). Hand-rolled, about 150 lines on the `bpe_simple_vocab_16e6` merges file, which removes the `onig` C dependency. The `tokenizers` crate also works as a fallback. |
| Vector search | Brute force over int8 vectors held in RAM. |

I did not pick the OpenAI ViT-B/32 weights. The code is MIT, but I found no explicit weights licence, and the model card marks deployed use out of scope. The LAION weights carry a clear MIT tag. Both share the architecture and the ONNX export recipe, so swapping is trivial.

## Rust inference options

| Option | Maturity | CLIP/SigLIP | Windows MSVC | Size impact | Licence |
|---|---|---|---|---|---|
| `ort` 2.0.0-rc.14 | Release candidate, but used in production (Wasmtime, SurrealDB, Magika) and 22M downloads. Wraps ONNX Runtime with MLAS int8 kernels. | Any ONNX export. Public ONNX CLIP/SigLIP exports exist, with int8 variants. | `load-dynamic` needs no build step; `download-binaries` also works. Windows ships an old `onnxruntime.dll` in System32, so load by absolute path from our folder. | `load-dynamic`: about +1 MB exe, DLL about 12 MB downloaded. Static link: probably +15-25 MB (unverified). | MIT OR Apache-2.0 |
| `candle` 0.11.0 (candle-core, candle-transformers) | Mature, pure Rust. Has `clip`, `siglip` and `mobileclip` (S1, S2 only) examples and models. | OpenAI CLIP, SigLIP, MobileCLIP S1/S2. TinyCLIP probably loads through the generic CLIP config (untested). | Easiest build (no C deps, no DLL). | Exe +5-8 MB, but weights are safetensors f32 or f16: B/32 is about 600 MB or 300 MB. No practical int8 for CLIP. | MIT OR Apache-2.0 |
| `tract-onnx` 0.23.9 | Pure Rust ONNX. Transformer export support is hit-or-miss, int8 is partial, and it is slower than ORT. | Unproven. | Easy. | Small. | MIT OR Apache-2.0 |
| `fastembed` 7.1.1 | A wrapper on `ort` plus `tokenizers` plus `hf-hub`. Image model `Qdrant/clip-ViT-B-32-vision` is the default (fp32, large). | Only its curated list. | Fine, but a heavy dependency tree. | Same as `ort`. | Apache-2.0 |
| Others | `onnxruntime` crates are dead; `burn` ONNX import is immature for ViTs. | | | | |

Verdict: `ort` with `load-dynamic` gives the best speed and smallest download (int8), with no change to the single-exe story for users who never opt in. `candle` is the fallback if the DLL step proves unreliable. Its cost is a larger model download and roughly 1.5-3x slower indexing (estimate). `fastembed` hides the interesting parts (int8, preprocessing, our cache path) and pins its own `ort` version, so I would not use it.

## Models

| Model | Params (img+text) | Download (int8 / fp16) | Dim | Licence | Verdict |
|---|---|---|---|---|---|
| OpenCLIP ViT-B/32 laion2b | 88M + 63M | about 89 + 64 MB int8 / 176 + 127 MB fp16 | 512 | MIT | Default. 4.4 GMACs per image, 49 patch tokens. |
| OpenAI CLIP ViT-B/32 | same | same (Xenova ONNX sizes) | 512 | Code MIT, weights unclear | Avoid on licence grounds. |
| TinyCLIP 22M/32 + 10M | 22M + 10M | about 35-45 MB int8 | 512 | MIT | Lite tier. 53.7% ImageNet at 1.9 GMACs. |
| TinyCLIP 40M/32 + 19M | 40M + 19M | about 60 MB int8 | 512 | MIT | 59.8% ImageNet, 3.5 GMACs. Marginal gain over B/32. |
| SigLIP base p16-224 (also SigLIP2) | about 0.2B | 94 + 111 MB int8 | 768 | Apache-2.0 | Too slow: 196 patch tokens, about 4x the cost of B/32 per image. |
| MobileCLIP / MobileCLIP2 S0-B | 11-86M + 42-63M | about 43 MB (S0) / 136 MB (S2) vision, fp32 community ONNX | 512 | Weights under Apple's research-only licence (AMLR) | Excluded. Fast (S0 about 1.5 ms on phone hardware) but "research purposes" excludes distribution in a tool. |

Quality on pixel art: CLIP-family models saw plenty of game art and pixel art on the web, so they should read material, colour and mood. Expect strong results for "lava", "snow", "grass", "bones", "water" and "dark red brick". Expect weak results for structural, tile-specific concepts ("wall corner facing north", "doorway frame"), because floor and wall tiles are fragments (a diamond with transparent corners, or a slice of a wall). Those are already served by the `orientation`, `class` and `idx:` filters. This is my reasoning, not a measurement; the spike exists to test it.

## Throughput and storage (estimates)

- Image embedding with ORT on a modern 8-core desktop CPU: about 15-25 ms per image fp32, about 8-12 ms int8 (batched; 4.4 GMACs per image). TinyCLIP 22M/32 roughly half of that.
- 26k MXL + 16k vanilla tiles = 42k. int8 B/32: about 6-8 min; fp32: about 12-17 min.
- All 90k assets: about 15-18 min int8, about 25-35 min fp32. Cut this with three measures:
  1. Embed by thumbnail content hash. Identical variants share one vector, and mirrored copies already cluster.
  2. Skip fully transparent thumbnails.
  3. For sprites, embed only one frame (direction 0, frame 0), and make sprites a second opt-in phase.
- Use `intra_op_threads = cores - 2` so the Bevy UI stays smooth. A paused or cancelled run resumes, because rows are committed in batches of 256.
- Storage: 512 dims as int8 plus a 4-byte scale is about 516 bytes per vector. 90k vectors is about 46 MB; f16 would be 92 MB, f32 184 MB. int8 per-vector scalar quantization costs a negligible amount of ranking quality. Deduplication shrinks this further.
- First-run cost for the user: one-time download of about 165 MB (B/32 int8 plus DLL) or about 55 MB (Lite), then the indexing run above. Nothing changes for users who do not opt in.

## Query path

- Text encoder (63M params): about 20-40 ms at the full 77 tokens. The ONNX text model can run at the true token length (CLIP pools at the EOS token with a causal mask), so a 6-token query is about 5-10 ms. Cache the vector per query string. Load the model lazily on first use, or when Ctrl+K opens (about 0.3-1 s for the int8 file), on a worker thread.
- Brute force: 90k x 512 int8 dot products is about 46M multiply-adds, around 3-8 ms in plain autovectorized safe Rust (no `unsafe` needed); f32 is about 10-20 ms. No ANN index is needed; HNSW only pays off past roughly 500k vectors and adds build code, memory and tuning.
- Filters first, then score: run the existing SQL filters (`type:`, `act:`, `color:` and so on) to get candidate ids, then score only those vectors. This composes naturally with every existing token.
- Total budget: debounce 150 ms, encode about 10 ms, search about 5 ms. Well under 100 ms after embeddings exist.

## Quality risks and mitigations (dark 96 px pixel art)

- Median luma is 40 of 255. Mitigation: composite transparency over neutral mid-grey, not black, which would dominate the embedding. Then apply auto-levels (stretch the 2nd-98th luma percentile to about 20-235) and optionally a gamma of about 0.7. The spike should compare raw, auto-levels and gamma on the same 100 tiles.
- Tiny source: upscale by the largest integer factor that fits 224 with nearest-neighbour (preserves crisp pixels, as web pixel art is usually shown), then letterbox to 224 x 224 with the mean colour. Do not stretch or centre-crop isometric diamonds. Compare against bicubic in the spike.
- Prompt templates: average the normalised embeddings of "a pixel art texture of {q}", "a top-down isometric game floor tile of {q}" and "a Diablo II {q} tile". Use wall and object templates when `type:` narrows the kind.
- Cosine scores are uncalibrated (about 0.2-0.3 for good matches). Rank only; show the top N or everything within 0.05 of the best score. Never show a "match percentage".
- Fuse with the existing text signals using reciprocal rank fusion of CLIP rank, FTS rank and nucleo rank. Generated names such as `Act1 Wilderness · Floor · grass` carry strong structural information that CLIP lacks.
- Image-to-image: CLIP cosine gives a semantic "similar" that pHash cannot, but pHash stays for exact and mirror matching. Expose the new one as its own token rather than changing the meaning of `similar:`.
- Evaluation gate: 30 queries over 100 hand-labelled tiles, precision@10 for CLIP vs FTS-only vs fusion. Proceed only if fusion beats FTS-only by a clear margin.

## Embedded weights vs opt-in install

| | Embed with `include_bytes!` | Opt-in install into `capybara\cache\clip\` |
|---|---|---|
| Exe size | +150 MB (B/32 int8) or +45 MB (Lite), plus a statically linked ORT (est. +15-25 MB) | about +1 MB |
| Single exe | Yes | Yes for the editor; the feature needs the cache files |
| Build | Slow link, huge CI artifact, MSVC `include_bytes!` of 150 MB is awkward | Normal |
| Updates | Every model change is a new exe | Swap files, bump `model_id` |
| Offline first run | Works | Needs a download or a manual drop-in |

Recommendation: opt-in install. It fits the PLAN rule because `cache\` is allowed and rebuildable. Provide two routes: a manual drop-in (copy four files into `capybara\cache\clip\<model_id>\`; zero network code, fully offline) as the MVP, and later a "Download (165 MB)" button using a small HTTPS client with SHA-256 verification against hashes pinned in the exe. If the files are missing or the hash differs, the feature is simply hidden.

## Implementation sketch

New crate `capy-embed` (keeps `ort` out of `capy-index` and out of compile times for everyone else), behind a cargo feature `semantic` in `capy-app`:

```rust
// capy-embed
pub struct ModelPaths { pub dll: PathBuf, pub vision: PathBuf, pub text: PathBuf, pub vocab: PathBuf, pub id: String }
pub fn locate(cache_dir: &Path) -> Option<ModelPaths>;   // verifies presence and hashes
pub struct Clip;
impl Clip {
    pub fn load(p: &ModelPaths, threads: usize) -> Result<Clip>;
    pub fn embed_images(&self, thumbs: &[Thumb]) -> Result<Vec<Vec<f32>>>;  // preprocess + L2-normalise
    pub fn embed_text(&self, query: &str) -> Result<Vec<f32>>;              // prompt-ensemble average
}
```

`capy-index` stays model-agnostic via a trait, which also makes tests easy (a fake embedder, matching the existing no-timing-benchmark policy):

```rust
pub trait Embedder: Send + Sync {
    fn model_id(&self) -> &str;
    fn dim(&self) -> usize;
    fn embed_images(&self, batch: &[(i64 /*asset_id*/, &[u8] /*thumb PNG*/)]) -> Result<Vec<Vec<f32>>>;
    fn embed_text(&self, q: &str) -> Result<Vec<f32>>;
}
impl Index {
    pub fn semantic_status(&self, profile: &str, model_id: &str) -> Result<SemanticStatus>;    // total, embedded
    pub fn embed_missing(&self, profile: &str, e: &dyn Embedder,
        on_progress: &dyn Fn(BuildProgress), cancel: &AtomicBool) -> Result<EmbedReport>;
    pub fn semantic_search(&self, profile: &str, qvec: &[f32], filter: Option<&Query>, limit: usize) -> Result<Vec<Hit>>;
    pub fn similar_visual(&self, profile: &str, key: &str, limit: usize) -> Result<Vec<Hit>>;
}
```

Storage: a separate file `capybara\cache\clip.db`, not a table in `index.db`. The profile index is dropped and rebuilt whenever an archive changes, and `SCHEMA_VERSION` mismatches wipe the file; a 15-minute embedding run must survive that. Schema:

```sql
CREATE TABLE embeddings(thumb_hash BLOB NOT NULL, model_id TEXT NOT NULL,
  scale REAL NOT NULL, vec BLOB NOT NULL, PRIMARY KEY(thumb_hash, model_id)) WITHOUT ROWID;
```

`thumb_hash` is a 64-bit hash of the thumbnail PNG bytes. `index.db` gains `assets.thumb_hash INTEGER` (schema v2, a `docs/api/capy-index.md` change) so assets map to vectors without rehashing, and rebuilds reuse every vector whose thumbnail did not change. At first semantic query, load all vectors for the profile into `Vec<i8>` plus ids (about 46 MB).

Background phase: after the normal index build finishes, a second "Semantic" phase starts, but only if models are installed and the user enabled the feature. Order: tiles, then sprites (sprites opt-in). It reuses the existing progress-bar plumbing, is cancellable and resumable, and ignores assets that already have a vector.

Query integration (`query.rs`, `search.rs`):
- Plain free text stays on nucleo and FTS, instant per keystroke, so no regression.
- New filter `like:"mossy stone wall"` (or a UI toggle that routes free text to CLIP) embeds the text, scores candidates and fuses with the FTS rank.
- `vsimilar:<key>` does CLIP image-to-image. `similar:` keeps pHash.
- Results flow through the existing `Hit` type, so thumbnails and the browser need no changes.

Hooks to reserve now (all cheap): store `thumb_hash` in the thumbnail pipeline; keep `Hit` constructible from an id list with a score; leave `like:` and `vsimilar:` unclaimed in the filter grammar.

## Effort

| Task | Days |
|---|---|
| Quality spike in Python (needs permission to download models) | 1 |
| ONNX export, int8 quantization, release hosting, hashes | 1 |
| `capy-embed` (ORT load, preprocessing, tokenizer, prompt ensemble) | 3 |
| `capy-index` (`clip.db`, `thumb_hash` schema v2, embed phase, search, tests with a fake embedder) | 2 |
| UI (settings opt-in, install instructions or download button, progress, Ctrl+K `like:`) | 2-3 |
| Total | about 9-10 dev days; with parallel agents about 3-4 calendar days |

## Risks

1. Quality on dark pixel art is unproven; the gate above de-risks it before any crate work.
2. `ort` is still `2.0.0-rc.x`. API churn between candidates is likely; pin the exact version. Each `ort` release maps to one ONNX Runtime version, and the DLL we host must match it (check the version-mapping page at install time).
3. DLL loading on Windows: an old `onnxruntime.dll` in System32 can shadow ours, so load by absolute path. Antivirus may flag a downloaded DLL; pin hashes and document the manual route.
4. Model hosting and licences: MIT for LAION weights, the ORT DLL and TinyCLIP; add all to `NOTICE`. Never ship Apple AMLR weights. The LAION data card warns the training set is uncurated, so surface a short note in the opt-in dialog.
5. Heavy CPU for 15-30 minutes: cap threads, pause on window focus or idle if needed, make it resumable.
6. `intra_op` threads plus Bevy's task pool contend; test on a 6-core machine.
7. `unsafe`: none in our code. `ort` and ORT use it internally.
8. Scope creep: fine-tuning on D2 tiles would help but is out of scope.

## Sources

- ort crate (versions, licence): https://crates.io/crates/ort and linking docs https://ort.pyke.io/setup/linking
- candle repo (models and examples: clip, siglip, mobileclip): https://github.com/huggingface/candle
- fastembed-rs: https://github.com/Anush008/fastembed-rs
- tract: https://github.com/sonos/tract
- Xenova CLIP ViT-B/32 ONNX file sizes: https://huggingface.co/Xenova/clip-vit-base-patch32/tree/main/onnx
- Xenova SigLIP base ONNX file sizes: https://huggingface.co/Xenova/siglip-base-patch16-224/tree/main/onnx
- LAION CLIP ViT-B/32 laion2B (MIT, 66.6% IN): https://huggingface.co/laion/CLIP-ViT-B-32-laion2B-s34B-b79K
- OpenAI CLIP licence discussion: https://discuss.huggingface.co/t/question-regarding-clips-model-open-sourceness-and-commerciality/67501
- TinyCLIP (MIT, model zoo): https://github.com/wkcn/TinyCLIP and https://huggingface.co/wkcn/TinyCLIP-ViT-39M-16-Text-19M-YFCC15M
- MobileCLIP / MobileCLIP2 model table and licences: https://github.com/apple/ml-mobileclip
- MobileCLIP2 ONNX community export: https://huggingface.co/plhery/mobileclip2-onnx
- Apple AMLR licence text (research purposes only): https://huggingface.co/apple/OpenELM-3B-Instruct/blob/main/LICENSE
- SigLIP base (Apache-2.0): https://huggingface.co/google/siglip-base-patch16-224 and SigLIP2: https://huggingface.co/google/siglip2-base-patch16-224
