# OverFilm

OverFilm is a private build of [FilmCraft](https://github.com/storytold/filmcraft) for editing
Overwatch videos, focused on accurate Korean captions (speech recognition) first. It is not
distributed. Everything in `README.md`, `AGENTS.md` and `CLAUDE.md` still applies (never crash,
pure Rust, everything is a command, exact time); this file covers only what OverFilm adds.

## Code layout

OverFilm code lives in its own crates so upstream FilmCraft updates merge cleanly. Changes to
upstream files are kept to small hook points (registering commands, choosing a transcriber,
calling the caption layout) and listed in the table at the end.

| Crate | Layer | Milestone | Purpose |
|---|---|---|---|
| `overfilm-glossary` | L1 | M2 | glossary tables (shared term ids, spellings, old names, misrecognitions) and their term-list export; transcript correction (`correct`), Korean spelling and spacing (`ko`), spoken numbers to digits (`digits`), in-game voice lines found by text (`voicelines`) |
| `overfilm-asr` | L2 | M3, M5 | Qwen3-ASR + Qwen3-ForcedAligner on candle, or on any Vulkan/DX12/Metal GPU through wgpu (`gpu/`: WGSL kernels, CPU fallback) (done: Q8_0 decoder weights, prompt-prefix cache, low-energy chunking, verified model download); speaker labels (`speakers/`: Kaldi filterbank, WeSpeaker ResNet34 on candle, average-linkage clustering, per-word labels, overlapped speech with pyannote segmentation-3.0); next: `filmcraft_speech::Transcriber` |
| `overfilm-captions` | L2 | M4 | Korean caption layout: two lines at most, display-width limits, penalty-based breaks (done), SRT in/out; caption styles and motion for the editor (`styles`), Premiere text styles and templates read into them (`premiere`) |
| `overfilm-vision` | L1 | M7 | screen recognition for esports broadcasts (break screen, round setup, pause templates; frozen pictures), the auto-cut plan, deaths and ultimates from the HUD's player bars, who killed whom and ultimate kills from the kill feed, heroes on the name plates (`heroes`) and teams and players from text read off the HUD (`lineup`); see "Auto cut" below |
| `overfilm-pipeline` | L3 | M4 | the caption pipeline OverFilmSubs and the editor share: built-in Overwatch pack (`pack`), recognition with correction, speakers and game voice (`recognise`, feature `qwen`), caption turns (`speakers`), filler clean-up (`clean`), enrolled voices (`voices`), the match prior and the built-in roster (`game`), HUD text reading (`ocr`, feature `ocr`), the shared models folder |
| `overfilm-eval` | L3 | M0 | transcript evaluation: CER, WER, game-term accuracy (library + `overfilm-eval` binary) |
| `overfilm-ai` | L1 | M6 | analysis behind the editing suggestions: loudness, silences, highlights (`overfilm.assist.suggest` in the engine), the learnt editing style (`style`) |
| `overfilm-review` | L1 | M6 | Claude Code review: runs the local `claude -p` (the user's subscription, no API key) on unclear cuts and captions; questions, JSON schemas and checked answers; see "Claude Code review" below |
| `overfilm-subs` (`apps/`) | L6 | — | standalone subtitle extractor: window (`OverFilmSubs`), command line and MCP review server (`overfilm-subs`); see its README |
| `packs/overwatch/ko/` | data | M2 | Overwatch Korean pack: `glossary/` (heroes, abilities, maps, modes, terms, teams, players; see its README), `voicelines.tsv` (hero ultimates and announcer as heard in the Korean client), later fillers, caption rules |

Layers follow `docs/architecture.md` §1 and are enforced by `cargo xtask layers`, where the
`overfilm-*` crates are listed by full name.

## Evaluation

Every accuracy decision (which model, which glossary rule) is made on numbers from
`overfilm-eval`. The reference videos and captions (M1) are kept outside the repository.

```sh
# one pair
cargo run -p overfilm-eval -- --terms terms.txt reference.srt hypothesis.srt
# two directories, files paired by name without extension; --json for the full report
cargo run -p overfilm-eval -- --terms terms.txt --json refs/ hyps/
```

- **CER** (headline for Korean): edits ÷ reference characters, on letters and digits with
  spacing removed, so `대지분쇄로` and `대지 분쇄로` are the same.
- **WER**: the same on eojeol (space-separated words); kept for comparison with published numbers.
- **Term accuracy**: share of the reference's game-term occurrences the hypothesis spells right.
  A term list has one term per line, `canonical | other accepted spelling`, `#` for comments;
  `--terms packs/overwatch/ko/glossary` builds it from the glossary instead.
- `--correct packs/overwatch/ko/glossary` first runs the glossary correction on each hypothesis
  (`overfilm_glossary::Corrector`: known misrecognitions, near-misses of names by jamo distance,
  profanity to initial consonants, then Korean spelling and spacing and spoken numbers to digits),
  to measure what the correction step adds.
  Matching ignores spacing and punctuation and takes the longest term at each position.
- Both sides are normalized the same way: caption markup removed, decomposed Hangul composed,
  full-width ASCII folded, digits read out in Korean (`6개` = `여섯 개`, `1위` = `일 위`),
  lowercase, punctuation as spaces. Corpus totals sum edits over files.

`crates/overfilm-eval/samples/` has a hand-counted example; its README lists the expected numbers.

## Recognition speed and piece length

`overfilm_asr::transcribe` cuts the sound at quiet points into pieces of at most 60 s
(`MAX_CHUNK_SECONDS`) and recognises them side by side: eight at a time on the GPU (`BATCH`),
eight on the CPU (`CPU_BATCH`):

- **Side by side**: generating text is one decoder pass per token, and that pass is bound by
  reading the 2 GB of 8-bit weights. The pieces of a group share each pass: on the CPU the 8-bit
  product reads every weight row once for all rows (`qmm.rs`; candle's own walks the weights once
  per row), and the prompt prefills of the group run as one product (`TextModel::prefill_seqs`);
  on the GPU the prompt prefix stays cached at the start of the key/value cache, each piece gets
  a region after it (`rope.wgsl`/`attn.wgsl` take a per-row position and region), and the
  `mv` kernel reads each weight once for up to eight rows (`Text::forward_seqs`). The
  recogniser's GPU cache holds 16,384 positions (`KV_CAP_SIDE`, about 1.9 GB of VRAM; the
  aligner keeps 8,192), so eight 60-second pieces fit; pieces that do not fit run in turns. Greedy decoding per piece
  is unchanged: on the GPU the same 65 s gave the same text, character for character, as the CPU.
- **A CPU decoding step** is bound by memory traffic, not arithmetic: the 2 GB of weights, then
  each piece's key/value cache (about 1,400 positions × 28 layers). So:
  - attention is a plain loop over the cache (`DecoderLayer::forward_rows`, parallel over piece ×
    key/value head; the two query heads of a key/value head read each position once). As
    separate candle operations (rotary, narrow, slice_set, two small matmuls, softmax) per piece
    and layer, the set-up cost was half of each step;
  - the cache is appended in place and kept in 16-bit floats (`Kv`; more precision than the
    model's own bf16), half the bytes per step;
  - the 8-bit product (`qmm.rs`) has its own block format (f32 scale + 32 values) and an AVX2
    kernel through `pulp` (safe SIMD; |w| and sign(x, w), unsigned × signed byte products in
    pairs, four accumulators); a scalar loop elsewhere;
  - eight pieces per weight pass on the CPU instead of four: the weights are read once either way,
    so each token costs less (the caches take about 170 MB per 60-second piece).
- **The encoder front** (three 3×3 stride-2 convolutions over the mel chunks) gathers the input
  patches and runs one matrix product per group of chunks (`conv3x3s2`); candle's tiled
  convolution took 7 s of a 60-second piece's 11 s in the encoder.
  On the GPU the same products run there (`gpu/front.rs`: `gather.wgsl` lays out the patches,
  applying the previous convolution's GELU as it reads, then the encoder's 16-bit product
  kernels), in passes of at most 256 MB of patches; only the positions and the per-chunk
  trimming stay on the CPU. Before, GPU mode still spent about 4 s of CPU time per 60-second
  piece here. `OVERFILM_GPU_FRONT=cpu` keeps the front on the CPU. `gpu-check` compares the
  whole encoder, front included, against the CPU.
- **Element-wise work on all cores**: candle runs element-wise operations on one thread. gelu
  (encoder), silu · up (decoder MLP) and the attention scores' scale, mask and softmax are one
  parallel pass each (`gelu`, `silu_mul`, `scaled_softmax`), and grouped-query attention
  multiplies the query heads of a group against their shared keys as one matrix instead of
  copying the keys per head (`attend_grouped`). Two 60-second pieces on 4 cores: encoder
  19.9 → 17 s, prompt pass 36.3 → 32.4 s, same captions.
- **Loading**: the 8-bit decoder weights are cached next to the model (`overfilm-q8.cache`,
  `qcache.rs`; about 1.9 GB for the 1.7B model) after the first load, keyed by each shard's name,
  size and time, and used by both the CPU and the GPU load (the GPU took the same blocks from
  its own quantizer). The remaining bf16 tensors convert to f32 in parallel (`weights.rs`).
  Loading the 1.7B model on the CPU went from about 22 s to 4 s. `OVERFILM_NO_Q8_CACHE=1`
  ignores the cache. Each run's stage times (`Transcript::stages`) are printed after a run.
- **Prompt passes on the GPU in 16-bit**: when the GPU has shader-f16, a second tiled product
  kernel (`tile16.wgsl`) keeps the 64×64 tiles in workgroup memory as f16 and multiplies with
  packed 16-bit math; every run of 16 products is added into f32 accumulators, and inputs are
  scaled by 1/16 (and clamped) into f16 so large activations cannot overflow. At start-up it is
  compared with the CPU (limit 2e-3, the 32-bit kernel 1e-3) and timed against the 32-bit tiles
  on a 512-row product; it is used only when it matches and is at least 10 % faster.
  `OVERFILM_GPU_PREFILL=f32` turns it off, `=f16` forces it (software renderers skip the timing).
  `gpu-check` prints which kernel prompt passes use. A subgroup version was not added: the
  tiles already share their inputs through workgroup memory, and subgroup sizes differ between
  drivers (RDNA runs 32 or 64 lanes), which would need one more kernel per size with no way to
  check it here.
- **Silence**: with `Settings::skip_silence`, quiet stretches of 2 s or more (30 ms frames below
  max(-55 dBFS, the 95th percentile level - 40 dB), kept with 0.3 s of margin;
  `chunk::split_voiced`) are not recognised at all; caption times stay those of the whole file.
  Commentary over in-game sound is rarely quiet that long, and a quiet game track can still hold
  speech, so audio embedded in a video (one track) keeps it off by default and the editor gets a
  warning when turning it on (`SKIP_SILENCE_WARNING`); separate audio tracks turn it on
  (`overfilm_pipeline::recognise::skip_silence_default`).
- **Piece length**: the models take up to 3 minutes (the reference toolkit uses 180 s pieces
  when it needs word times). Longer pieces give the recogniser more of the sentence around each
  word and fewer cuts inside one. M1 aichi, CPU, same build:

  | piece | CER | WER | substitutions | terms (video list) |
  |---|---|---|---|---|
  | 30 s | 6.60 % | 14.59 % | 120 | 99.1 % |
  | **60 s (chosen)** | **6.25 %** | **13.16 %** | **101** | 97.3 % |
  | 120 s | 6.20 % | 13.49 % | 102 | 96.4 % |

  120 s leaves room for only one piece at a time in the GPU cache, so 60 s. Runs differ by about
  ±0.2 % CER from rounding alone, so the term column (one or two words) is within noise. With
  two spelling rows found on the same video (`딜라인` → `뒷라인`, `배나는` → `밴하는`): 6.18 % CER,
  13.05 % WER, 97 substitutions (before this work: 6.56 %, 14.37 %, 120).
- **Time** for the 12:45 aichi video on 4 CPU cores (`--speakers 1`, `target-cpu=x86-64-v3`):
  about 38 minutes before (text generation 0.30 s per token, 68 % of the time) → 24.4 minutes
  with four pieces side by side (0.16 s per token of each piece) → 18.8 minutes with the cache,
  attention, product and batch changes above (0.075 s per token: 3161 tokens of eight pieces in
  237 s). Same video: 6.03 % CER, 12.83 % WER (two spelling rows from it: `꼴을 빨` → `꿀을
  빨`, and `개월` as a number counter, `육 개월` → `6개월`). Where the time goes now (CPU): model
  loading about 50 s, encoders about 11 s per minute of sound for the recogniser and 7.5 s for
  the aligner, prompt prefill about 20 s per piece and the aligner's pass about 14 s per piece
  (both dense f32 products, bound by arithmetic), text generation about 400 s. Not timed on a real GPU (the build container has none); there
  token passes are bound by memory reads, which the side-by-side pieces share.

## Speakers and game voice

`overfilm_asr::speakers` labels who spoke each recognised word; `apps/overfilm-subs/src/speakers.rs`
adds game-voice detection and builds captions turn by turn (`overfilm_captions::layout_turns`:
no caption mixes two speakers, a labelled turn starts with `(화자1)`).

- **Voice vectors**: WeSpeaker ResNet34 trained on VoxCeleb (the pyannote release,
  `pyannote/wespeaker-voxceleb-resnet34-LM`, CC-BY-4.0, downloaded at first use and pinned by
  SHA-256 in `catalogue::SPEAKER`). Input is an 80-band Kaldi filterbank written from the Kaldi
  documentation (`speakers/fbank.rs`; matches `kaldi-native-fbank` to 2e-4). The network runs
  once per 10 s block (blocks every 8 s) and any window of the block is pooled from that frame
  map, as pyannote does. Checked against the published ONNX export: cosine 1.0000.
- **Clustering**: 1.5 s windows every 0.5 s inside speech (words joined across pauses under
  250 ms, so windows rarely span a turn change), average linkage on cosine distance (NN-chain,
  at most 2500 points, more are subsampled), cut at 0.7, windows reassigned to the nearest
  centroid, speakers under 10 s of speech (or 8 % of it) folded into the nearest one. A speaker
  count from the user replaces the threshold.
- **Tuning set**: synthetic conversations from the Zeroth-Korean test set (CC-BY-4.0, 10
  speakers): 24 conversations of 2–5 speakers, 150 s each, a quarter of the turns cut to
  0.5–1.2 s interjections, 80–600 ms gaps, light noise. Word-level speaker error after the best
  label mapping, and speaker count:

  | setting | word error | count right |
  |---|---|---|
  | threshold 0.6, min 3 s, windows may cross 400 ms pauses | 30.9 % | 0/24 |
  | threshold 0.7, min 6 s, windows stop at 250 ms pauses | 4.0 % | 16/24 |
  | **threshold 0.7, min 10 s (chosen)** | **3.4 %** | **24/24** |
  | threshold 0.75, min 10 s | 10.3 % | 16/24 |

  | **+ per-word centroid scores and Viterbi turns (switch cost 0.4)** | **1.9 %** | **24/24** |

  The generator and scorer are kept with the M1 evaluation files (outside the repository).
- **Turns**: each word scores every speaker by the similarity of the windows over it to the
  speaker centroid; a Viterbi pass charges 0.4 (similarity × seconds) for a speaker change inside
  fluent speech and almost nothing after a sentence end or a 400 ms pause. On the T1 vs 로제
  clip this cut speaker changes in the middle of a sentence (no punctuation, gap under 200 ms)
  from 24 to 1.
- **Enrolled voices**: `Diarizer::profile` averages a sample's 1.5 s window vectors (quiet
  windows skipped); `apps/overfilm-subs/src/voices.rs` saves it as `voices/<name>.json` beside
  the models. After clustering, `match_known` gives each enrolled name to the nearest centroid
  within cosine distance 0.55 (closest pairs first, one each), and an enrolled speaker is never
  folded away as too small. `--only-voices` keeps only enrolled speakers' words (streamer mode).
  Synthetic check (24 conversations, every speaker enrolled from a different 10 s utterance of
  the same person): every word gets a name, 97.9% the right one, 2.1% a wrong one.
- **GPU**: `gpu/conv.wgsl` runs every convolution of the network (one dispatch each, one
  submission per block, only the last frame map read back). `Diarizer::use_gpu` compares GPU and
  CPU frame maps on a 2 s test block first (relative error ≤ 1e-3) and stays on the CPU
  otherwise; a GPU error on a block falls back to the CPU for that block. `gpu-check` includes a
  small residual network against a plain-loop reference.
- **Overlapped speech** (`speakers/seg.rs`, `speakers/overlap.rs`): pyannote segmentation-3.0
  (MIT). The original repository needs a Hugging Face sign-in, so `catalogue::SEGMENT` pins the
  ONNX export onnx-community publishes openly (6 MB); only its weights are read (a bounded
  protobuf reader), the graph is written out in Rust (SincNet, 4 × BiLSTM 128, powerset
  classifier). Matches onnxruntime to 5e-5 on real audio (same class in every frame), 0.2 s per
  10 s chunk on one CPU core. After the words have speakers, 10 s chunks every 5 s give each
  frame up to three local talkers; a local talker is tied to a transcript speaker by the word
  labels under its single-voice frames (12+ frames, 60 % of them one speaker), or else by its
  voice vector (WeSpeaker over its frames, no clean ones needed) against the voices of tied
  talkers, never to a speaker already tied in that chunk. A word whose frames are at least 20 %
  overlapped (two-talker probability ≥ 0.3, averaged over the chunks holding it, centre-weighted)
  gets the other voice as `Word::also`. Voice vectors are computed only in chunks with an untied
  talker in an overlap, which keeps the cost near the segmentation's.
  Tuning set: 12 synthetic Zeroth conversations of 2–4 speakers in which 30 % of turns start
  0.3–2 s before the previous one ends and 15 % are backchannels said wholly inside another turn
  (301 overlapped words: a word counts as overlapped when another speaker is audible, by that
  speaker's own energy, in 30 % of its span; counting every word inside another turn's time
  span, pauses included, gave 359 and the lower 65 % / 55 % first reported):

  | setting | overlap words found | both speakers right | false overlap |
  |---|---|---|---|
  | most likely class per frame, 30 % of the word (old truth) | 49.3 % | 43.7 % | 1.1 % |
  | **probability ≥ 0.3 over chunks, 20 % of the word (chosen)** | **75.1 %** | **64.5 %** | **2.8 %** |
  | probability ≥ 0.2, 15 % of the word | 78.1 % | 67.1 % | 4.2 % |
  | voice ties need similarity 0.4 (not 0.2) | 75.1 % | 63.5 % | 2.8 % |

  Hero set: the same generator with one speaker of every conversation replaced by a held-out
  hero's Korean voice lines (Soldier: 76; 405 overlapped words). Ties carried between
  neighbouring chunks (`stitch`: a talker untied in one chunk takes the tie of the talker it
  matches, by shared talk ≥ 0.5 over the 5 s both chunks see) raised it from 72.6 % found /
  53.3 % both right / 4.4 % false to **80.0 % / 61.5 % / 4.9 %**, the people-only set unchanged.
  Fine-tuning the segmentation on Zeroth train (105 speakers) mixed with five other heroes'
  lines (3000 steps, CPU) did not help: found 73.6 %, both right 50.6 % on the hero set, so the
  published weights stay.
  Clean pooling (`OverlapParams::clean_pool`, `Diarizer::run_with_overlaps`): the chunks are
  segmented before the speaker vectors, and 80 ms frames whose two-talker probability reaches
  0.7 are left out of each window's vector (a window keeping under half its frames, or under
  0.4 s, gets none), so one voice is not pooled with another. Word error / overlap found / both
  right / clean-word error, before → after:

  | set | before | clean pool 0.7 | clean pool 0.5 |
  |---|---|---|---|
  | people (12, overlaps) | 8.9 / 75.1 / 64.5 / 5.2 % | **8.4 / 78.7 / 68.1 / 4.8 %** | 8.3 / 78.7 / 68.1 / 4.7 % |
  | Soldier (12) | 10.6 / 80.0 / 61.5 / 5.7 % | **9.3 / 79.3 / 63.0 / 4.8 %** | 9.9 / 73.8 / 56.8 / 5.5 % |
  | 12 other heroes, one per case | 9.6 / 56.0 / 49.0 / 5.1 % | **9.1 / 54.5 / 48.0 / 4.9 %** | 12.4 / 53.0 / 43.0 / 8.2 % |

  On the 24 people-only conversations (no planned overlaps) word error goes 2.5 → 2.2 % and
  the right speaker count 22 → 23 of 24.

  Tried and dropped: keeping a small speaker whose centroid is far (cosine ≥ 0.75–0.9) from
  every other (no gain on Soldier, 10 → 7 right counts on the other heroes), and leaving the
  hero's words out of the vectors even with the true spans (people word error 5.2 → 5.0 %: the
  hero lines barely disturb the people's voices; they are found by `voicelines.tsv` instead).
  | + relabel words by the segmentation's clean talkers | — | — | word error 4.8 → 5.5 %, dropped |

  The word's own label is right (one of the two talkers) for 97 % of overlapped words. On the
  T1 vs 로제 clip a quarter of the words are overlapped: the streamer talks over the broadcast
  casters. Captions show `(화자1·화자2)` only for a steady overlap (two or more words, 1 s or
  more, same pair); a person's own file (`--per-speaker`, `--only-voices`) keeps the words said
  over someone else; a word of a game voice with a person talking over it that is no voice line
  goes to the person. Without the model the step is skipped.
- **Game voice**: `overfilm_glossary::voicelines` matches runs of words against
  `voicelines.tsv` on letters only (edit distance at most a fifth of the line). An exact match of a line
  of 5+ characters (score ≥ 0.8 for lines of 8+) marks its words as game audio; a speaker with 2+ matches whose
  words are at least half voice lines becomes a game voice as a whole; near misses of short lines
  are ignored, and only ultimate/announcer/skin lines or lines of 8+ characters count on their own;
  communication-wheel lines (kind `comm`, "방어구가 필요해") and short everyday lines ("아니거든요")
  only count towards a speaker. The table holds ~20,000 lines: ultimates and announcer for most
  heroes, plus the Korean lines of 40 heroes taken from the client's sound file names (lines of
  under 5 characters, glossary words such as Echo's copied hero names, foreign-language and
  event-mode lines left out; Anran, D.Mon, Domina, Shion and Sierra still missing).
  `find` compares only lines sharing enough character pairs with the text (an edit removes at most
  two distinct pairs): ~60 ms for 1,000 words. On 40 recognised hero lines 19 are marked on their
  own; on two real streams no streamer words are. Captions drop game audio
  by default (`--game-voice label|keep` to show it). Recognising game audio by sound rather than
  text (hero voices, effects, music) is not done yet.

## Auto cut (esports broadcasts)

`Sequence ▸ OverFilm ▸ Auto Cut Waiting and Pauses` (`overfilm.autoCut`) removes the waiting
time of an Overwatch tournament broadcast, by the picture only (no audio):

- **Break / standby screens** (big countdown, sponsor panel) and **round setup** ("PREPARE TO
  ATTACK/DEFEND" with its timer) are cut up to `keepBefore` (10 s) before they end, i.e. before
  the match or round starts. A break screen running straight into a setup (gap ≤ 3 s) is one wait.
- **Pauses** (a pause notice, or a frozen picture: ≥ 97 % of 8×8 blocks unchanged by more than
  one gray level for 4 s or more) are cut whole; the join gets a dissolve and an audio
  crossfade. A frozen picture counts only inside a round (after a setup ended, before the next
  break screen), or, once the set has a game-HUD (`live`) template, only while the HUD is on screen.
- **Nothing else is cut.** A frame no template matches is play; a cut shorter than 2 s is dropped.
- A frozen picture that runs straight into a setup (the in-game scoreboard between halves) is part
  of that wait. Game-HUD (`live`) templates never compete with the others: the HUD is on screen
  during setups and pauses too, so they only say "the game is on screen".

**Measured on a real broadcast** (OWCS Asia Korea Stage 3, LCQ day 2, first 84 minutes at 720p:
pre-show, map 1 Nepal, map 2 Route 66, map 3 Colosseo, two breaks). Templates: the break screen's
left panel, the objective texts PREPARE TO ATTACK / PREPARE YOUR DEFENSES / GET READY / ASSEMBLE
YOUR HEROES, and the top bar's logo as the HUD. 7 cuts, 21:40 removed: the pre-show countdown, both
breaks and the setups of Nepal, Route 66 and its second half (with the scoreboard before it), each
ending 10 s before the round starts; every cut edge checked by eye, no play cut. Missed: the setup
of a Push map (Colosseo), whose HUD shows only the countdown and "0.00m" (also shown after the round
starts), so it has no text to learn. No pause happened in the clip, so pauses are only tested on
generated video. The run is decoding bound: the clip's file is opened once more per core (up to 4
decoders) and each decodes 2-minute chunks, put back in time order so the samples are exactly
those of one decoder (the frozen-picture check at a chunk's start compares with the previous
chunk's last frame), and each template compares a list of its cells made once per frame. On
4 cores (dev build) this took the 84 minutes from 1373 s to 769 s with the
same segments and cuts (`overfilm_autocut_e2e_on_a_broadcast`, ignored test).
`cargo run -p overfilm-vision --release --example autocut_raw` runs templates and the plan on raw
gray frames piped from ffmpeg (see its header).

Screens are recognised by **templates** learnt from a frame of the same broadcast
(`overfilm.screenTemplate.learn {screen: standby|setup|pause|live, rects, ignore, time}`):
each rectangle (frame-relative, 0..1) is kept as a gray grid of at most 128 cells and found again
by normalised cross-correlation (half-cell steps, ±1 % of the frame width); `ignore` leaves out
what changes (the running clock, a camera feed). A template matches when its worst rectangle
correlates at 0.7 or more. The set is a JSON file: `templates` when given, else one beside the
project (`<project>.overfilm-screens.json`) when it exists, else the user's own
`overfilm-screens.json` beside the OverFilmSubs models (`%LOCALAPPDATA%\OverFilm`), shared by every
project and usable before the project is saved. `overfilm.screenTemplate.import {path}` (the
**Import Templates…** button in the Auto Cut and Learn dialogs) merges someone else's file into it,
e.g. a broadcast's templates learnt once. The user's file starts with the built-in OWCS Asia Korea
2026 templates and name plates (`crates/overfilm-vision/data/`, see its README): built in at the
owner's request for this private build; they derive from the league's graphics and the game HUD, so
they must not go into a public release.

The analysis samples 2 frames a second at up to 1280 pixels wide in a background job (the HUD's
objective text is about 8 pixels tall at 720p). By default
(`mode: "mark"`) each planned cut becomes a range marker "OverFilm 컷: 대기 / 퍼즈" for review;
`overfilm.autoCut.apply` then removes the ranges of the markers left (moved or deleted ones
included) in one undo step. `mode: "cut"` does both at once.

## Korean captions in the editor

`Sequence ▸ OverFilm ▸ OverFilm Captions (Korean)` (`overfilm.captions.generate`) makes the
captions inside the editor with the same pipeline as OverFilmSubs (`overfilm-pipeline`): Qwen3-ASR
with the Overwatch glossary as context, word times, glossary correction (profanity as initials,
spelling, digits), speaker labels (enrolled voices named) and in-game voice told apart. It runs in a
background job on the media of the sequence's audio clips (or `items`); the models are downloaded
on first use into the folder OverFilmSubs uses (`%LOCALAPPDATA%\OverFilm\models`), and the user
glossary (`…\OverFilm\glossary`) applies too. Options: `mode` (general / commentary: team and
player names as context), `speakers`, `numSpeakers`, `extraTerms`, `gpu`, `gameVoice` (drop /
label / keep), `lineChars`, `maxLines`, `leadMs` (120: shown this long before the voice), `holdMs`
(400: kept this long after it), `readingCps` (12: kept up long enough to read at this many letters
a second when the next caption leaves room), `cleanFillers`, `audioTrack`. Lines and captions
break at natural Korean boundaries (sentence and clause endings, particles, pauses), never inside
a phrase such as `할 수 있다` or `하고 있다`. The editor app builds with
it by default (`apps/filmcraft` feature `overfilm-qwen`).

- `audioTrack` (1 = A1; the dialog's "Audio track") recognises only the clips on that track, each
  with the audio stream and source channels the clip plays: a recording with the microphone on a
  stream of its own (OBS) lands on A2 when dropped in, so choosing A2 captions the voice alone. Only
  those clips' media are captioned. OverFilmSubs has the same choice for files with several audio
  tracks (`--audio-track N`, MCP `audio_track`).
- `numSpeakers` up to 32 is kept as given (the automatic count stops at 8). `numSpeakers: 1` skips
  telling voices apart altogether (WeSpeaker and the overlap model do not run): faster, and the
  captions carry no labels anyway.
- The time left is estimated from the average speed between the first and the latest step of the
  recognition (`Progress::set_steady_pace`), not the last 15 seconds: recognition moves a piece of
  sound (up to 30 s) at a time, and a 15 s window swung between "20 minutes" and "2 hours".

- Each clip's transcript is stored like any other (Text panel ▸ Transcript): media-time words,
  speakers 화자1, 화자2…, in-game voice as the speaker "게임 음성". Text-based editing, search and
  `transcript.*` commands work on it.
- The captions go on the caption track "OverFilm 자막" (`overfilm.captions.fromTranscript`, which
  also works alone on any transcript, Whisper or imported): at most two lines with the Korean
  breaks, one turn per speaker, `(화자1)` labels when two or more people talk, game voice left out
  by default. Making them again replaces that track. Because the words are taken through the clips
  as they are now, running it after an Auto Cut or any edit gives captions that fit the cut.
- Then `Styled Captions` and `Export for Premiere Pro` below take that track.
- Checked end to end on 60 s of a Korean Overwatch video (`overfilm_e2e_recognises_a_real_recording`,
  ignored by default: `OVERFILM_E2E_MEDIA=clip.mp4 cargo test -p filmcraft-engine --features
  overfilm-qwen -- --ignored overfilm_e2e`): the editor's captions are the same, cue for cue, as
  OverFilmSubs' SRT of the same clip.
- Overlapped speech (`화자1·화자2` turns) is not carried into the project transcript, which has one
  speaker per word; OverFilmSubs still writes those.

## Styled captions and Premiere Pro

Captions made in the editor (above), or an SRT from OverFilmSubs (`File ▸ Import`), sit on a caption
track. Then:

- `Sequence ▸ OverFilm ▸ Styled Captions` (`overfilm.captions.stylize {preset, motionIn,
  motionOut, track}`) makes one graphic clip per caption on a video track "OverFilm 자막 ·
  <style>", in one undo step (styling again replaces that track). Each clip is an ordinary
  graphic text layer, so fill, strokes, box, shadow, position and keyframes stay editable in
  Effect Controls. The caption track stays, output off, for SRT export.
- **Styles** (`overfilm_captions::styles`, original designs): 기본 (white, black outline),
  박스 (white on a dark box), 예능 노랑 (yellow, black and white double outline, pop), 해설 두 사람
  (a colour per speaker: 화자1 sky blue, 화자2 orange, 화자3 green; slides up), 강조 (big red,
  middle of the frame, bounce), 한 글자씩 (typewriter). A leading `(화자1)` label written by
  OverFilmSubs picks the speaker's colour and is dropped from the text.
  `overfilm.captions.savePreset` adds the user's own (`<project>.overfilm-caption-styles.json`,
  or the user's `overfilm-caption-styles.json` for an unsaved project).
- **Premiere styles** (`overfilm_captions::premiere`, `overfilm.captions.importStyle {path}`; the
  dialog's **Import Premiere Style…**): the user's own `.prtextstyle` (Premiere: right-click a
  caption style ▸ Export Text Style), `.mogrt`, `.prproj` or Premiere XML becomes OverFilm styles
  in the user's `overfilm-caption-styles.json` (beside the OverFilm models, shared by every
  project). From a Premiere text style: font, size, fill, outline, background box (colour,
  opacity, size, corner radius) and shadow on/off; fonts are matched to installed families and
  the ones not installed are reported. From an After Effects `.mogrt`: the first text control's
  font and size (scaled to 1080 lines) and the colours its control names place (text, stroke,
  background; a gradient's start colour as the fill), and its **motion**: the `.aep` inside
  (`overfilm_captions::aep`, a bounds-checked RIFX walker) gives the text layer's scale,
  position, opacity and rotation keyframes; the keys before the longest still stretch become
  the caption's custom motion in, the keys after it the motion out (`CaptionPreset::custom_motion`,
  values relative to the rest pose, eased or linear per key), sped up so in and out take at most
  60 % of a caption. A per-character text animator becomes the typewriter. The text is measured
  through its parents and the precompositions that show it, and expressions are evaluated
  (`overfilm_captions::aep_expr`: variables, if/else, Math, ease/linear/clamp, effect controls,
  other layers' transforms, valueAtTime, loopOut; `wiggle()` becomes the Wiggle effect).
  The rest of the look (`overfilm_captions::aep_look`, `CaptionPreset::template`,
  `styles::caption_clip`): Glow → Wonder Glow, CC Light Sweep → **Text Light Sweep** (OverFilm's own
  effect), Gaussian/Fast Blur, Directional Blur, Drop Shadow → the text's
  shadow, Gradient Ramp → a gradient fill, Fill → the fill colour; their keyframes fitted into
  each caption like the motion. Masks go on the caption clip's opacity; rectangle/ellipse shape
  layers become shape layers (pinned to the text when sized to it); the other text layers' motion
  moves the caption's second and later lines. Not carried over (listed in the import notes):
  `sourceRectAtTime()`/`random()`/loops-in-code expressions, third-party plug-ins, free-form
  path shapes, 3D/cameras/lights, track mattes and layer styles. Mask and effect positions are
  measured from the text's centre at rest, so they are close but not exact. A template with no
  readable keyframes falls back to a motion guessed from its names. Choosing a motion in the
  dialog replaces the template's motion (its effects stay).
  The FlatBuffers layout was worked out from text styles
  people share publicly (14 `.prtextstyle` and 8 `.mogrt` files all read); unknown fields are
  skipped. No Adobe file is shipped or read by us: the user picks their own files. Upstream's
  `.fcgt` templates still refuse `.mogrt`; this is a separate OverFilm reader.
- **Save Selected Caption's Look** (`overfilm.captions.styleFromClip {name, clip?}`): a caption
  restyled in Effect Controls is saved back as a style (`styles::preset_from_layer`).
- **Motion** in and out: fade, pop (scale 70 → 108 → 100), slide up, bounce, typewriter (Source
  Text held per character, at most 60 ms each). Motion lasts `motion_secs` (0.2 s), at most a
  third of the caption.
- `File ▸ Export ▸ Export for Premiere Pro (OverFilm)` (`overfilm.premiere.export {dir}`) writes
  `<name>.xml` (FCP7 XML of the sequence without the styled track: Premiere's File ▸ Import),
  `<name>.srt`, `<name>-자막.mov` (the styled captions alone, ProRes 4444 with alpha, from 00:00
  to the last caption, to lay on the top track at 00:00) and a short how-to in Korean. Adobe's
  `.mogrt` is not written (closed format).

## Editing suggestions

`Sequence ▸ OverFilm ▸ Suggest Silences and Highlights` (`overfilm.assist.suggest {silence,
highlights, count=10, seconds=20, thresholdDb=-45, minSilence=1}`) mixes the sequence's sound in a
background job (10 loudness blocks a second, `overfilm_ai`) and marks, for review:

- **Silences**: below -45 dBFS for 1 s or more after 0.2 s is left on each side, never over a
  transcript word, not inside waiting time Auto Cut already marked. They are "OverFilm 컷: 무음"
  markers, so `Apply OverFilm Cut Markers` removes them together with the waiting time.
- **Highlights** (green "OverFilm 하이라이트 N", 20 s, the peak at two thirds): each second scores
  how far it is above the median loudness of the minute around it (3 dB free), plus 4 points per
  hype word in the transcript ("미쳤다", "전멸", "나이스", "역전"…, `HYPE_WORDS`) and 1.5 per
  in-game voice word (ultimates); smoothed over 5 s, the highest non-overlapping peaks win. The
  marker comment says why ("점수 23 · 소리 +9 dB · 미쳤다, 전멸 · 게임 음성 3단어").

Measured on the 84-minute OWCS broadcast by sound alone (`cargo run -p overfilm-ai --example
assist_raw`, no transcript): of 12 highlights, 4 were team fights in play, the rest the pre-show
(caster laughter, the intro video) and the loud bumpers between rounds. Hence two filters in the
editor: no highlight inside Auto Cut's waiting markers, and, when the screen-template file has a
game-HUD (`live`) template, a highlight must show the game in at least two of its peak and 2 s either side.
Silences: an esports broadcast has hardly any (3, 3 s in all); they matter for talking videos.

**Highlight reel**: `Sequence ▸ OverFilm ▸ Make Highlight Reel` (`overfilm.highlights.reel {name?,
dissolve=true, dissolveFrames?, open=true}`) copies the sequence as "<sequence> 하이라이트" and
ripple-deletes everything outside the highlight markers on every track, captions included (locked
and not sync-locked tracks too; their settings come back afterwards), so clips keep their effects,
keyframes and links. The moments stay in time order, overlapping markers join, and each join gets
a short cross dissolve and crossfade where both sides have media. Delete or move the highlight
markers first to choose which moments go in; the source sequence is not changed.

## Kills and team fights

The broadcast HUD's **player bars** give every death steadily: each player's name plate is in the
team colour while they are alive and turns black while they wait to respawn
(`overfilm_vision::kills`). The kill feed (small boxes over a moving game, colours washed out by the
stream) is harder to read; it adds who killed whom (see "Who killed whom" below).

- `overfilm.playerBar.learn {team: 1|2, rect: [x, y, w, h], players=5, time?, clip?, templates?}`
  (or the Learn Screen Template dialog with "Team 1/2 name plates"): the rectangle around one team's
  plates, on a frame where the whole team is alive; it is split into `players` equal plates and
  saved in the screen-template file (`bars`), beside the templates.
- `Sequence ▸ OverFilm ▸ Find Kills, Ultimates and Team Fights` (`overfilm.kills.scan {fps=2, clips?,
  templates?, fightGap=10, minKills=3, wait}`): a background job reads the plates while the game
  HUD (`live`) template matches. A plate reads dead when over 35 % of it is near black (the white
  name stays bright) and must read so for half a second. A plate also goes black for a few seconds
  while its player changes hero; then the white health segments stay under it, where a dead
  player's plate has a red cross instead, so a black plate over white segments (and no red) is not
  a death; samples where most plates are dark at
  once (replays, transitions) are skipped. Each death becomes a point marker
  "OverFilm 처치: N팀 M번" (team colour) and each run of 3+ deaths no more than 10 s apart a range
  marker "OverFilm 교전 N" (5 s before the first death to 3 s after the last). Running it again
  replaces them.
- `overfilm.assist.suggest` adds 5 points per death to the highlight score ("처치 N" in the
  comment), so a fight stands out even when the casters stay calm.

Measured on the 84-minute OWCS broadcast (bars learnt at 30:58, `overfilm_kills_e2e_on_a_broadcast`,
2 frames a second, 19 minutes on a CPU): no deaths outside play (the pre-show and breaks fail the
HUD template), about 147 deaths and 23 team fights over the three maps. Checked against the frames:
the fights at 31:06 and 34:30 (a team wipe) died in the order and at the times the plates show,
and two of the four sound-only highlights (31:06, 1:15:19) fall on fights. A plate that flickers back
mid-respawn made a few double deaths, hence no second death of a player within 8 s. Hero changes
were counted as deaths until the strip under the plates was read: over 30:00 to 1:00:00 of that
broadcast they were 11 of 112 "deaths" (each a black plate for about 3 s over the health segments).

The scan's time goes into decoding the video: every frame of a stream has to be decoded to reach
the sampled ones (that broadcast's stream has no frames nothing refers to; on recordings that have
them, the scan now skips them). After decoding, the gray picture comes straight from the video's
Y' plane, and only the kill feed and the strips under the plates are made into colour. Two minutes
of the 720p broadcast take about 14 s on a 4-thread CPU (the decoder already uses the threads;
decoding stretches at once on more decoders saved a few seconds there for four times the memory,
so it is not done).

## Ultimates and ultimate kills

Each name plate also has a small box beside the player's picture: the ultimate charge as a number
("43%") and, once the ultimate is ready, a round white icon, the same on every plate of a team
(`overfilm_vision::ults`).

- `overfilm.ultIcon.learn {team: 1|2, rect: [x, y, w, h], player=1, time?, clip?, templates?}` (or
  the Learn Screen Template dialog with "Team 1/2 ready ultimate icon" and the plate's number): a
  box round one ready icon, on a frame where that player's ultimate is ready. Saved with the team's
  bar (`bars[].ult`: the icon's gray grid placed on the first plate, how much of it is bright disc,
  and its median gray). The built-in OWCS file has both teams' icons.
- `overfilm.kills.scan` (above, with `ultWindow=6`) then follows the icons too, in the same pass
  over the frames (the deaths it finds do not change). A plate reads ready when its box correlates
  with the icon (0.75+) and shows at least 75 % of the icon's bright disc (a small blur first, so
  the thin charge digits do not count). A plate that read ready for a second and then not, while
  the player is alive and the plate is still on screen (its box keeps 60 % of the icon's median
  gray: a dead plate turns black, and the bars leave the screen at the end of a round), is a use:
  a point marker "OverFilm 궁극기: N팀 M번" (violet). A second use within 10 s, or one followed by
  the player's own death within 1.5 s, is left out.
- Without a kill feed to read, an ultimate kill is by timing: a death of the other team no
  more than `ultWindow` seconds after a use, given to the latest such use: "OverFilm 궁극기 처치:
  N팀 M번 ← N팀 M번 궁극기" (yellow; the comment says it is an estimate). Healing and movement
  ultimates followed by a kill count too.
- On broadcasts that show the game's kill feed, `overfilm.killFeed.learn {rect, icon, time?, clip?,
  templates?}` (dialog: "Kill feed (ultimate kills)", first green box around the feed, second round
  one ultimate kill's icon) learns where it is and how big the icons are (`killfeed`; nothing of the
  icon is stored). The kill rows of an ultimate carry the ultimate's icon before the arrow: a white
  disc whose rim ends at the same distance from its centre in nearly every direction (portraits'
  hair and a scope's reticle do not). An icon that stays two samples and was not there (rows move up
  and down, not sideways) is an ultimate kill, marked "OverFilm 궁극기 처치: N팀 M번 ← … (킬피드)"
  with the death on the plates 1 s before to 2.5 s after it when there is one.

Measured on the 84-minute OWCS broadcast (built-in icons, learnt at 34:40; `ults_raw` example,
2 frames a second): 104 ultimate uses, and every one checked against the frames was right (34:28
two at once, 35:03, 35:12, 35:16, 35:17, 35:42 two at once): the ready icon was on the plate and the
charge number or the hero's running-ultimate icon replaced it. Before the on-screen check, 12 false
uses came from the bars leaving the screen at the ends of rounds. Its 49 timing-based ultimate kills
were counted before the kill feed was read. The kill-feed reading found exactly the three ultimate
kills of six rows in a kill feed screenshot the owner sent (no portrait or reticle taken for one),
and the three ultimate kills of the broadcast fights checked below.

### Who killed whom (kill feed)

On videos that show the game's kill feed (gameplay, streams, broadcasts that keep it), the kill
scan reads every row once the feed is learnt (`overfilm_vision::feedrows`); `icon` is optional for
this, `rect` around the feed is enough, and the feed's row height is learnt with it. The built-in
OWCS file has that broadcast's feed (`[0.74, 0.185, 0.26, 0.30]`, rows 0.125 of it high; its top
starts above where the feed sits under the kill count, as the broadcast drops the count in some
shots and the feed moves up).

- A row is two boxes in the team colours (blue: the viewer's team, red: the other) with the killer's
  hero picture after the first and the victim's before the second; a row with one box is a death
  with nobody to credit. Rows that touch (a reticle or an effect between them) are cut apart at the
  emptiest line near each row height. The ultimate icon is read on the row itself, between the two.
- Nothing built in is matched: the white name text (a gray grid of its own width) and the hero
  picture are kept as small grids, and the same name or picture read again is the same player or
  hero (normalised correlation of the grids moved by a cell or two, names of lengths more than
  12 % apart never match). They are numbered as they first appear: "선수 1", "영웅 2".
- A row on screen for two samples that was not there before is one kill, at its first sample.
  Rows are followed from sample to sample by where their boxes start and end (in row heights; a
  name blurred by motion or the stream reads a little wider or not at all, its box stays put),
  their sides and names, keeping the rows' order (a new row comes in at the top and pushes the
  others down; a new top row must not take the place of the old one read just below it). A row
  that ends well left of the feed's right edge (the median of where victims' boxes end) is
  something else. A row missed for a sample or two (a flash, a smear) is the same row when it is
  found again near where it was; one lost for longer is the same kill when its boxes are where they
  were and its names alike. A killer's box missed on a sample does not make another row.
- Players and heroes are named from the sharpest readings of the row (up to 8 of each side, by
  how sharp the name text is), and a player keeps up to 24 readings of the name, so the next
  reading is compared with all of them (one name blurs differently every time). Each kill is a point marker
  "OverFilm 킬피드: 선수 1 (영웅 1) → 선수 2 (영웅 3)" (" (궁극기)" with the icon; blue killer cerulean,
  red rose, nobody tan), the comment "아군 → 적". Frames are decoded up to 1920 wide while the
  feed is read. With no name plates' deaths on the sequence the highlights count these kills.
- `overfilm.killFeed.list {}`: the kills (kill feed markers, else the plates' deaths) with time,
  clock and text, and the last scan's players and heroes. The OverFilm panel shows it under
  "처치 목록": a click moves the playhead there.
- `overfilm.killFeed.name {kind: "player"|"hero", id, name, templates?}` (the panel's "선수·영웅
  이름 붙이기"): names player or hero `id`; the markers are written again (one undo step) and the
  name and its grid go into the template file (`feed_names`), so later scans name the same text or
  picture themselves. Names are the user's: nothing ships with any.

Measured on the owner's kill feed screenshot (six rows): all six read right at its size, at 2/3 and
at 1/2 of it (NECROS four times and FRANKYBOXING and LEEVI twice each, one player each; three
ultimate kills); at 1/2 one victim's picture was taken for another hero's. The same rows played as
a 2 fps H.264 video (CRF 30), one row more every two seconds, gave the six kills at the right times
(`feed_raw` example: `rows`, `pairs`, `run`).

On the OWCS broadcast (720p, built-in feed, 2 fps), three team fights checked frame by frame
(33:49 to 34:02, 34:30 to 34:47 and 36:25 to 36:33, 14 kills): 13 found with the right teams and
within a second or two of the row's arrival (one, the last of the first fight, missed), all three
ultimate kills with their icon, and one kill counted twice when its row was lost for a few seconds
behind a camera cut. Who is who is weaker there: the names are about 8 pixels high and blurred, so
two names of a length (WUTIAN, FAITH, GAMJUNG) can share one "선수 N" until named. Ten minutes of
the broadcast read in about 30 seconds on a CPU, decoding with ffmpeg included.

## Teams, players and heroes on screen

The kill scan (`overfilm.kills.scan`, `players=true` by default) also reads who plays what off the
name plates, on the frames it decodes anyway, and OverFilm Captions uses it as a **soft prior**
for team, player and hero names.

- **Heroes** (`overfilm_vision::heroes`): each plate's hero portrait (right of the ultimate box;
  `portrait` on a player bar when a broadcast puts it elsewhere) as a 6×6 colour grid, compared
  scaled to its brightness (dark portraits differ in few levels). A new portrait counts once it has
  stayed 12 s: a death grays the portrait for the respawn time, and the ultimate animation covers
  it for a moment. The first hero of each stretch of play (between breaks of 2 s or more off the
  game HUD) is a pick, later changes are swaps. Heroes are numbered ("영웅 3") until named in the
  OverFilm panel ("이름판 영웅 이름 붙이기") or with `overfilm.heroes.name {id, name}`; the name and
  grid go into the template file (`hero_names`), so later scans name the portrait themselves.
- **Teams and players** (`overfilm_vision::lineup`, text read by `overfilm_pipeline::ocr`: the
  ocrs models, about 12 MB, downloaded once into the models folder): the team name over each bar
  and the names under the plates, read 2.5× enlarged every 2 s at the start of a stretch until each
  plate has 6 readings, then every 30 s. Readings are only compared with the glossary's roster
  (`players.tsv` sections under each team's heading; coaches and streamers left out): a side's
  team is the one its readings match best overall (`D2 Blat` still votes for O2 Blast), and its
  players are matched per plate per stretch against that team only, each player on one plate, so
  substitutes and plates reordered between rounds are followed. One unread plate whose team is
  otherwise unchanged is the player left over.
- Markers (replaced by every scan): "OverFilm 경기: 로제 vs 오투 블래스트"; "OverFilm 선수" where the
  plates change (comment: "로제: 태종, 킬로, …" per team; "OverFilm 선수 교체: 로제 A → B" for a
  substitute); "OverFilm 영웅 선택" at each stretch's picks (comment "태종: 트레이서" per line, only
  plates whose hero changed); "OverFilm 영웅 교체: 우톈 트레이서 → 겐지" at each swap.
- **Captions**: `overfilm.captions.generate` reads those markers back (hand-corrected markers
  count) into the clip's `MatchPrior`, moved through the clips of the media to its own clock.
  The teams and players go first in the recogniser's context ("이번 경기: 로제 대 오투 블래스트.
  출전 선수: …", before the long lists that the length cap cuts), and in the glossary correction the
  names in play (and the named heroes in play during each piece of sound) win a tie with other
  names and allow one more jamo edit at three syllables or more. Nothing else changes: a hero
  nobody plays is corrected as before, since commentators talk about other heroes too. A
  two-syllable player still needs `선수` after it to be corrected.
- `overfilm.heroes.list {}`: the last scan's teams, players and heroes (number, name, first pick).

Measured on the OWCS broadcast (720p, built-in template file, 2 fps, the 84 minutes there are):
both teams right (RØDE ZANSIDE GAMING, O2 Blast), all ten players on every plate of all six
stretches, including Kilo and Taejong trading plates between maps (checked on frames), in 135
readings (2½ minutes of a 10-minute run on 4 CPU cores, decoding included). 14 swaps, 37 picks;
the ones checked on frames are real (30:26 Heesung, 1:10:39 Homerunball, five O2 players
changing heroes in the spawn room before 45:03). Before the brightness scaling, dark portraits of
different heroes shared one number. No substitute in this video, so substitutes are tested only
on made-up readings (`lineup_tests`). Evaluation tools: `heroes_raw` (vision) and `lineup_raw`,
`recognise_raw` (pipeline) examples.

Captions on 10 minutes of commentary (28:20–38:20, CPU, speaker split off), with the prior from
that scan against without: "오토플라스트" became "오투 블래스트", "오브러/오픈" became "오프너"
(1 → 5), "일리어" became "에일리언", "로드호그 게이밍" went away. The correction alone
(`correct_raw`, same recogniser output) changes nothing but those names. The longer context also
moves some unrelated words ("홈런볼" → "홈런 볼" twice, one "우텐" heard as "오투"), and WuTian is
still often 우티언/우티안, so the prior helps team and player names most where they are spelled
nearly right already.

## Claude Code review

Off until switched on in `Sequence ▸ OverFilm ▸ Claude Code Review Settings…`
(`overfilm.review.settings {enabled, auto, cliPath, model, timeoutSeconds=180, batch=6,
maxItems=60}`, saved as `claude-review.json` in the OverFilm data folder). When on, the editor hands
the calls it is unsure of to the Claude Code command line on the same computer:

- **Getting ready**: asking for a review (`overfilm.review.cuts`, `overfilm.review.captions`, or the
  one-go run with a review step) checks Claude Code first. Not installed: an install guide opens
  with the commands for this system (native installer, npm, `claude auth login --claudeai`) and
  Copy buttons (`overfilm.review.install`). Review off, not signed in, or signed in with an API key:
  the settings open with what to do and a Sign In button (`overfilm.review.login` starts
  `claude auth login --claudeai` in its own window). The model is picked from a list
  (`overfilm.review.models`): Claude Code's default, the aliases the installed version's `--help`
  names for `--model`, opus/sonnet/haiku, and the user's default from `~/.claude/settings.json`, or
  typed. A good check is trusted for ten minutes. The review commands stay in the menu while review
  is off, so they can lead the user there.

- **How it runs** (`overfilm_review::cli`): one `claude -p --output-format json --json-schema …
  --tools "" --no-session-persistence --strict-mcp-config` run per batch of questions, the prompt on
  stdin, in an empty temporary folder. `ANTHROPIC_API_KEY` and `ANTHROPIC_AUTH_TOKEN` are removed
  from its environment, so it always uses the Claude account the user signed in to Claude Code
  with (their subscription). Windows: `claude.exe` (native install) or the script behind an npm
  `claude.cmd`, no console window. A time limit per question (killed after it), the Jobs panel's
  stop button kills it too. Not installed, not signed in, usage limit, timeout or an answer that
  does not parse: a toast says which, nothing changes. `Check Claude Code` in the dialog runs
  `claude --version` and `claude auth status`.
- **Unclear cuts** (`Ask Claude Code About Unclear Cuts`, `overfilm.review.cuts {markers?, all,
  again, rules?}`): "OverFilm 컷" markers with speech within 1.5 s of an edge or inside, game voice
  inside, or a highlight or team fight ("OverFilm 교전") within 5 s. Each goes with the words heard
  30 s around it (speaker, game voice, before/inside/after) and what the screen templates see at five points. The answer per
  marker is `cut` (the reason added to the comment), `keep` (renamed "OverFilm 유지 (Claude)", blue,
  so Apply OverFilm Cut Markers skips it) or `partial` with exact keep ranges in ms (the marker
  becomes cut markers for the rest and keep markers for those; pieces under 0.5 s are dropped).
  Keep ranges are clipped into the marker. The user's rule goes with every question: waiting time up
  to 10 s before the start, pauses whole, silences only when nothing happens, nothing else.
- **Captions** (`Ask Claude Code About Unclear Captions`, `overfilm.review.captions {track?,
  captions?, all, again, mode?, lineChars=16, maxLines=2}`): captions with unsure words (confidence under
  0.7) or not-quite-sure ones (under 0.9: the recogniser is often sure of a wrong word that sounds
  alike), a glossary correction (the pure recognition differs from the corrected text), over the
  line limits, a one- or two-letter line, or a very short caption after them. Each goes with the
  caption, the pure recognition of its words (kept from recognition: `Recognised::raw_texts`,
  lined up with the corrected words by edit distance; beside a saved project as
  `<project>.overfilm-raw.json`), the corrected text, and the two captions before and after with
  their own pure recognition. Answers: `keep`, `fix` (new text, at most two lines) or `split`
  (2–4 captions; times cut at the word boundary where each part's letters run out). A fix far from
  both the caption and the pure recognition (over 40 % of the letters changed) or over the line
  limits is dropped; speaker labels such as `(화자1)` are kept. Each question also carries the kind
  of video (`mode`, or commentary when match info markers exist), the match info markers so far
  ("OverFilm 경기/선수/영웅 선택/영웅 교체") and the glossary terms its captions mention (spelling,
  other names, known misrecognitions, role). The instructions ask to fix confident misrecognitions
  that make no sense in context with a similar-sounding word or term, and to keep when nothing in
  the sound or the context supports a change. Measured on the aichi set (2026-11-02, real
  Claude Code, `cargo run -p overfilm-pipeline --example review_eval`): CER 5.97 %
  before review; old instructions 5.83 %; with confident misrecognitions asked about 5.68 %; with the
  hero roster and roles and the "same sound first" rule 5.41 % (172 of 307 captions asked, terms 98.2 →
  99.1 %); asking every caption (before the roster) 5.62 %. A `misheard` form marked `@검토`
  (`경제` for `경쟁`) is never replaced by the correction step; a caption with one is asked about
  and the form goes with it as a likely misrecognition.
- **On its own** (`auto`): after Auto Cut or Suggestions mark cuts, and after OverFilm Captions
  places captions, the matching review starts by itself.

Every review is one undo step; a marker or caption changed while it ran is left alone, and
reviewed ones are not asked again unless `again`. Measured with Claude Code 2.1 (default model):
about 6–8 s for one question. `cargo test -p overfilm-review -- --ignored` asks the real
Claude Code one cut and one caption.

## Cut and caption in one go

`Window ▸ OverFilm` (or `Sequence ▸ OverFilm ▸ Cut and Caption in One Go…`) opens the OverFilm
panel (`crates/ui-egui/src/panels/overfilm_panel.rs`). It docks, tabs and floats like any other
panel and stays in the saved workspace. It runs the ticked steps one after another
(`overfilm.flow.run`, `crates/engine/src/overfilm_flow.rs`):

1. **Auto Cut** (`overfilm.autoCut`, always as markers first), with the template file and the time
   kept before the start.
2. **Claude Code on the unclear cuts** (`overfilm.review.cuts`), when ticked.
3. **The cuts applied** (`overfilm.autoCut.apply`), unless the result is "markers to check first";
   markers already on the sequence (an earlier Auto Cut, Suggestions) are reviewed and applied too.
4. **Kills, ultimates and players** (`overfilm.kills.scan`, `killFps`), on by default (`kills`):
   it reads the teams, players and heroes off the HUD before the captions, so they get the names
   (see "Teams, players and heroes"). Left out when the template file has no name plates or kill
   feed; a scan that fails is noted and the run goes on without the names.
5. **Captions**: recognised (`overfilm.captions.generate`, audio track, video kind, speakers,
   in-game voice), or laid out again from the transcript the sequence already has
   (`overfilm.captions.fromTranscript`: after the cuts, without a second recognition).
6. **Claude Code on the captions** (`overfilm.review.captions`), when ticked.
7. **Render** (`render`), when ticked: the finished sequence goes out with `file.exportMedia` in
   the chosen `renderFormat` (MP4 H.264/H.265/AV1, MOV ProRes/DNxHR, GIF, WAV, M4A), captions burnt
   in unless `burnCaptions` is off, or with `overfilm.premiere.export` (`renderFormat: "premiere"`:
   FCP XML + SRT for Premiere Pro). `renderPath` is a folder or a file; empty puts it beside the
   project. A broadcast day with several matches (different teams, the "OverFilm 경기" markers the
   kill scan writes) goes out as one file per match by default (`renderSplit: "match"`), named
   `<date>-<team 1>vs<team 2>.<ext>` (`<date>-경기<n>` when no team was read), each from its
   marker to the next so nothing between matches is dropped; a single match is one file under
   that name, a sequence with no match marker keeps its own name. `renderSplit: "one"` writes the
   whole sequence as one file.

Each step waits for the one before (its background job and its result applied). What can be
known at the start is refused at the start: no step chosen, a review ticked while Claude Code
review is off, "from the transcript" without one, no captions in this build, a template file
that can't be read, the first step's own errors. A later step that fails stops the run; a review
that fails is noted and the run goes on. While a run is going, the reviews that the "on its own"
setting starts after Auto Cut and OverFilm Captions are left to the run, so nothing is asked
twice. The panel shows each step (waiting, running with its progress and time left, done,
skipped, failed, how long it took); `Stop` (`overfilm.flow.stop`) stops the running job and the
rest. `overfilm.flow.status` gives the same over the control channel and MCP. Undo takes back each
step on its own. Below the run, buttons open the other OverFilm tools, grouped in the order of the
work: get ready (screen templates), cut (Auto Cut, suggestions, Claude Code on the cuts, apply),
read the match (kills, highlight reel), captions (captions, Claude Code on them, styles) and export
(media, Premiere Pro).

## Editing style (beta)

`Sequence ▸ OverFilm ▸ Learn Editing Style (Beta)` (`overfilm.style.learn {sequences?, style?}`) learns
from finished edits: select them in the Project panel (a Premiere Pro edit comes in through
File ▸ Export ▸ Final Cut Pro XML in Premiere, then File ▸ Import here; `.prproj` is not read), or
it takes the open sequence. Per sequence it reads (`overfilm_style::facts`): the time between video
edit points (shots) and cuts per minute; how long one source plays on before a jump (consecutive
clips continuing the same source count as one stretch) on the busiest video track; transitions and
their length; added effects (not the built-in Motion / Opacity / Volume) and zooms (keyframed
Motion scale or position); how much of the sources is kept, when their length is known; caption
length, duration and lines. `overfilm_ai::style::learn` sums it up as quartiles and rates, saved as
`overfilm-style.json` (beside the project when that file exists, else the user's own) with a Korean summary (`overfilm.style.show` gives it back).

With a saved style the suggestions follow it: highlights run as long as the upper quartile of
the kept stretches (5–60 s) unless `seconds` is given, and the highlight reel joins moments with
plain cuts when fewer than a fifth of the editor's cuts carry a transition, else with a dissolve
of their usual length (unless `dissolve` / `dissolveFrames` is given). Checked by a round trip
(the demo edit exported as FCP7 XML, imported, learnt: same cuts, shots and transitions); not yet
on a real Premiere edit: a beta until it is.

## Dialogs

The Sequence ▸ OverFilm menu items open a dialog when chosen from the menu (`crates/ui-egui/src/panels/overfilm.rs`;
called with parameters, over the control channel or MCP, the command runs directly):

- **OverFilm Captions (Korean)…**: video kind, in-game voice, speaker count, characters per line,
  lines, extra terms, speakers / fillers / GPU switches.
- **Styled Captions (OverFilm)…**: caption track (the "OverFilm 자막" track first), style, motion in
  and out.
- **Auto Cut Waiting and Pauses (OverFilm)…**: template file, time kept before the start, frames
  per second, markers or cut right away, frozen picture as pause.
- **Learn Screen Template (OverFilm)…**: the Program frame at the playhead; drag on it to mark what
  always looks the same on that screen (green) and, switched to ignore, what changes (red: the
  clock, a camera). OK learns it into the template file. "Team 1/2 name plates (kills)" learns a
  player bar instead (the first green rectangle, and the number of players); "Team 1/2 ready
  ultimate icon" a team's ultimate icon (the first green rectangle, and the plate it is on); "Kill
  feed (who killed whom, ultimates)" the kill feed (the first rectangle around it, the optional
  second round one ultimate icon).
- **Cut and Caption in One Go (OverFilm)…** opens the OverFilm panel (above).
- **Claude Code Review Settings (OverFilm)…** (`panels/overfilm_review.rs`): on/off, on its own,
  `claude` path, model, time limit, batch size, most items, and a connection check.

The labels are in English in the code and Korean in `crates/ui-egui/src/i18n/ko.tsv` (Edit ▸
Language ▸ 한국어).

## Name and icon

The program shows itself as **OverFilm**: the window title (taskbar, Alt-Tab), the exe's
VERSIONINFO (`FileDescription`/`ProductName`, which Task Manager and Explorer show), message boxes,
the About dialog and the Korean labels that named FilmCraft. The icon is the OverFilm clapperboard
(`assets/app-icon/README.md`), used by the editor and by OverFilmSubs. Folder and file names
(`FilmCraft Previews`, `FilmCraft Media Cache`, `filmcraft.log`, the settings folder, `.fcproj`)
and the app id keep FilmCraft's, so existing settings, caches and projects carry over. The Windows
installer (NSIS, project files `overfilm-editor/installer/overfilm.nsi`) installs `OverFilm.exe`
with the same icon on the Start menu and desktop shortcuts.

Cross-compiling for Windows from Linux, `apps/filmcraft/build.rs` and `apps/overfilm-subs/build.rs`
embed the icon and VERSIONINFO with MinGW's `x86_64-w64-mingw32-windres` (`apps/filmcraft/winres.rs`;
`apt-get install mingw-w64`). Without it the exe builds without them, with a warning.

## Upstream updates

The first commit was a snapshot of filmcraft `fed0f42` without its history. The merge commit
`M0.1: link upstream filmcraft history at fed0f42` records `fed0f42` as a parent (tree unchanged),
so upstream updates now merge normally:

```sh
git remote add upstream https://github.com/storytold/filmcraft   # once per clone
git fetch upstream
git switch -c upstream-sync origin/main
git merge upstream/main          # conflicts should only touch the hook points below
```

Open a PR from the branch, and run the full gate (below) on it before merging.

Syncs so far:

| Date | Upstream | Notes |
|---|---|---|
| 2026-11-02 | v0.16.0 (`8c171a0a`) | `Marker` gained `url`, `author`, `custom_color` (OverFilm markers leave them empty); the steady pace in `crates/export/src/pace.rs` sits beside upstream's new batch-aware window |

## CI

`.github/workflows/overfilm.yml` is OverFilm's CI. On every push to `main` and every PR it runs
`cargo fmt --check`, clippy (`-D warnings`), tests and a wasm check on the `overfilm-*` crates,
`cargo xtask layers`, `cargo xtask assets` and the sample evaluation. The whole FilmCraft gate
(`cargo xtask ci`, about an hour) runs only when started by hand: Actions → OverFilm → Run
workflow → full.

The upstream workflow files (`ci.yml`, `nix.yml`, `sonarcloud.yml`, `freebsd.yml`,
`packaging-lint.yml`, `release.yml`, `windows-arm64.yml`) and `dependabot.yml` are left as they
are so upstream merges stay conflict-free. In this private repository they use storytold's shared
workflows and secrets and would spend Actions minutes, so disable them in the Actions tab
(each workflow → "…" → Disable workflow) and turn off Dependabot version updates in
Settings → Code security; dependency updates come in through upstream merges.

## Upstream files changed by OverFilm

| File | Change |
|---|---|
| `Cargo.toml` | `overfilm-*` entries in `[workspace.dependencies]` |
| `xtask/src/main.rs` | `overfilm-*` crates in `LAYERS`; layering also checks `overfilm-*` dependencies |
| `.github/workflows/overfilm.yml` | new file (OverFilm CI) |
| `OVERFILM.md` | new file (this document) |
| `crates/engine/Cargo.toml` | depends on `overfilm-vision`, `overfilm-captions`, `overfilm-pipeline`, `overfilm-asr`, `overfilm-ai`, `overfilm-review`; feature `overfilm-qwen` |
| `crates/engine/src/lib.rs` | `mod overfilm_autocut`, `mod overfilm_captions`, `mod overfilm_transcribe`, `mod overfilm_assist`, `mod overfilm_reel`, `mod overfilm_kills`, `mod overfilm_style`, `mod overfilm_review`, `Session::overfilm_jobs`, `overfilm_caption_jobs`, `overfilm_assist_jobs`, `overfilm_kill_jobs` and `overfilm_review`, their `poll` in `poll_persistence` |
| `crates/engine/src/media_pool.rs` | `MediaPool::reopen`: an uncached second opening of a file the host streams, for Auto Cut's parallel decoders |
| `crates/engine/src/commands.rs` | registers `overfilm_autocut::commands()`, `overfilm_captions::commands()`, `overfilm_transcribe::commands()`, `overfilm_assist::commands()`, `overfilm_reel::commands()`, `overfilm_kills::commands()`, `overfilm_style::commands()` and `overfilm_review::commands()` |
| `crates/export/src/pace.rs`, `lib.rs` | the steady pace (`Pace::set_steady`) for speech recognition's long, even steps |
| `crates/engine/src/transcript.rs` | `targets` is `pub(crate)` (which clips to recognise) |
| `crates/engine/src/overfilm_transcribe.rs` (+ `_tests.rs`) | new file: Korean captions in the editor |
| `crates/engine/src/overfilm_assist.rs` (+ `_tests.rs`) | new file: silence and highlight suggestions |
| `crates/engine/src/overfilm_reel.rs` (+ `_tests.rs`) | new file: the highlight reel |
| `crates/engine/src/overfilm_kills.rs` (+ `_tests.rs`) | new file: player bars, deaths and team fights, ultimates and ultimate kills |
| `crates/engine/src/overfilm_style.rs` (+ `_tests.rs`) | new file: the learnt editing style |
| `crates/engine/src/overfilm_review.rs` (+ `_tests.rs`) | new file: Claude Code review of cuts and captions; Auto Cut's `mark`, Suggestions' `make` and OverFilm Captions' `poll` call its `after_*` hooks, OverFilm Captions keeps the pure recognition for it |
| `crates/engine/src/overfilm_flow.rs` (+ `_tests.rs`) | new file: cut and caption in one go; `Session::overfilm_flow`, its `poll` after the other OverFilm polls; the review's `after_*` hooks leave a running one alone |
| `crates/ui-egui/src/panels/overfilm_panel.rs` | new file: the OverFilm panel; `PanelKind::OverFilm` in `dock.rs`, `panels.rs`, `app/window.rs` (opens beside Properties) |
| `crates/ui-egui/src/panels/overfilm_review.rs` | new file: the Claude Code review settings dialog (`route`/`show` next to `overfilm`'s) |
| `apps/filmcraft/Cargo.toml` | feature `overfilm-qwen`, on by default |
| `crates/ui-egui/src/panels/overfilm.rs` | new file: the OverFilm dialogs |
| `crates/ui-egui/src/panels.rs`, `menus.rs`, `panels/dialogs.rs` | `mod overfilm`, its `route` before the menu dialogs, its `show` |
| `crates/ui-egui/src/i18n/ko.tsv` | Korean labels of the OverFilm menu items and dialogs (appended) |
| `assets/app-icon/*`, `apps/filmcraft-web/web/favicon.png`, `ATTRIBUTION.md` | the OverFilm icon in place of the owl (same file names) |
| `apps/filmcraft/build.rs`, `apps/filmcraft/winres.rs` | exe icon and VERSIONINFO named OverFilm; `windres` when cross-compiling |
| `apps/filmcraft/src/main.rs` | `APP_NAME` ("OverFilm") for the window title and message boxes |
| `crates/ui-egui/src/panels/dialogs.rs` | About heading "OverFilm", "Based on FilmCraft" |
| `crates/ui-egui/src/i18n/ko.tsv` (values) | Korean labels say OverFilm where they said FilmCraft, except the FilmCraft website links |
| `crates/engine/src/overfilm_autocut.rs` (+ `tests.rs`) | new file: Auto Cut commands (frames in, cuts out); `dissolve_at` is shared with the reel |
| `crates/engine/src/overfilm_captions.rs` | new file: styled captions, Premiere styles in, and the Premiere Pro export |
