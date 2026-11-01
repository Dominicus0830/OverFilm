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
| `overfilm-captions` | L2 | M4 | Korean caption layout: two lines at most, display-width limits, penalty-based breaks (done), SRT in/out |
| `overfilm-eval` | L3 | M0 | transcript evaluation: CER, WER, game-term accuracy (library + `overfilm-eval` binary) |
| `overfilm-ai` | L4 | M6, M7, M9 | MCP editing assistants, screen recognition, style learning |
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
  heroes, plus the Korean lines of 39 heroes taken from the client's sound file names (lines of
  under 5 characters, glossary words such as Echo's copied hero names, foreign-language and
  event-mode lines left out; Anran, D.Mon, Domina, Shion, Sierra and Zenyatta still missing).
  `find` compares only lines sharing enough character pairs with the text (an edit removes at most
  two distinct pairs): ~60 ms for 1,000 words. On 40 recognised hero lines 19 are marked on their
  own; on two real streams no streamer words are. Captions drop game audio
  by default (`--game-voice label|keep` to show it). Recognising game audio by sound rather than
  text (hero voices, effects, music) is not done yet.

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
