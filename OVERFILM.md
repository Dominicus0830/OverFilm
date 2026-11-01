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
| `overfilm-captions` | L2 | M4 | Korean caption layout: two lines at most, display-width limits, penalty-based breaks (done), SRT in/out; caption styles and motion for the editor (`styles`) |
| `overfilm-vision` | L1 | M7 | screen recognition for esports broadcasts (break screen, round setup, pause templates; frozen pictures), the auto-cut plan and deaths from the HUD's player bars; see "Auto cut" below |
| `overfilm-pipeline` | L3 | M4 | the caption pipeline OverFilmSubs and the editor share: built-in Overwatch pack (`pack`), recognition with correction, speakers and game voice (`recognise`, feature `qwen`), caption turns (`speakers`), filler clean-up (`clean`), enrolled voices (`voices`), the shared models folder |
| `overfilm-eval` | L3 | M0 | transcript evaluation: CER, WER, game-term accuracy (library + `overfilm-eval` binary) |
| `overfilm-ai` | L1 | M6 | analysis behind the editing suggestions: loudness, silences, highlights (`overfilm.assist.suggest` in the engine), the learnt editing style (`style`) |
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
generated video. The run takes about 6 minutes for 84 minutes of video (decoding bound).
`cargo run -p overfilm-vision --release --example autocut_raw` runs templates and the plan on raw
gray frames piped from ffmpeg (see its header).

Screens are recognised by **templates** learnt from a frame of the same broadcast
(`overfilm.screenTemplate.learn {screen: standby|setup|pause|live, rects, ignore, time}`):
each rectangle (frame-relative, 0..1) is kept as a gray grid of at most 128 cells and found again
by normalised cross-correlation (half-cell steps, ±1 % of the frame width); `ignore` leaves out
what changes (the running clock, a camera feed). A template matches when its worst rectangle
correlates at 0.7 or more. The set is a JSON file beside the project
(`<project>.overfilm-screens.json`) or given as `templates`; the repository ships none, since the
league's graphics and the game HUD are not ours to redistribute.

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
label / keep), `lineChars`, `maxLines`, `cleanFillers`. The editor app builds with it by default
(`apps/filmcraft` feature `overfilm-qwen`).

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
  `overfilm.captions.savePreset` adds the user's own (`<project>.overfilm-caption-styles.json`).
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

Reading the kill feed itself proved unreliable on a compressed broadcast (small boxes over a moving
game, colours washed out). The broadcast HUD's **player bars** are steadier: each player's name
plate is in the team colour while they are alive and turns black while they wait to respawn
(`overfilm_vision::kills`).

- `overfilm.playerBar.learn {team: 1|2, rect: [x, y, w, h], players=5, time?, clip?, templates?}`
  (or the Learn Screen Template dialog with "Team 1/2 name plates"): the rectangle around one team's
  plates, on a frame where the whole team is alive; it is split into `players` equal plates and
  saved in the screen-template file (`bars`), beside the templates.
- `Sequence ▸ OverFilm ▸ Find Kills and Team Fights` (`overfilm.kills.scan {fps=2, clips?,
  templates?, fightGap=10, minKills=3, wait}`): a background job reads the plates while the game
  HUD (`live`) template matches. A plate reads dead when over 35 % of it is near black (the white
  name stays bright) and must read so for half a second; samples where most plates are dark at
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
mid-respawn made a few double deaths, hence no second death of a player within 8 s.

## Editing style

`Sequence ▸ OverFilm ▸ Learn Editing Style` (`overfilm.style.learn {sequences?, style?}`) learns
from finished edits: select them in the Project panel (a Premiere Pro edit comes in through
File ▸ Export ▸ Final Cut Pro XML in Premiere, then File ▸ Import here; `.prproj` is not read), or
it takes the open sequence. Per sequence it reads (`overfilm_style::facts`): the time between video
edit points (shots) and cuts per minute; how long one source plays on before a jump (consecutive
clips continuing the same source count as one stretch) on the busiest video track; transitions and
their length; added effects (not the built-in Motion / Opacity / Volume) and zooms (keyframed
Motion scale or position); how much of the sources is kept, when their length is known; caption
length, duration and lines. `overfilm_ai::style::learn` sums it up as quartiles and rates, saved as
`<project>.overfilm-style.json` with a Korean summary (`overfilm.style.show` gives it back).

With a saved style the suggestions follow it: highlights run as long as the upper quartile of
the kept stretches (5–60 s) unless `seconds` is given, and the highlight reel joins moments with
plain cuts when fewer than a fifth of the editor's cuts carry a transition, else with a dissolve
of their usual length (unless `dissolve` / `dissolveFrames` is given). Checked by a round trip
(the demo edit exported as FCP7 XML, imported, learnt: same cuts, shots and transitions); not yet
on a real Premiere edit.

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
  player bar instead (the first green rectangle, and the number of players).

The labels are in English in the code and Korean in `crates/ui-egui/src/i18n/ko.tsv` (Edit ▸
Language ▸ 한국어).

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
| `crates/engine/Cargo.toml` | depends on `overfilm-vision`, `overfilm-captions`, `overfilm-pipeline`, `overfilm-asr`, `overfilm-ai`; feature `overfilm-qwen` |
| `crates/engine/src/lib.rs` | `mod overfilm_autocut`, `mod overfilm_captions`, `mod overfilm_transcribe`, `mod overfilm_assist`, `mod overfilm_reel`, `mod overfilm_kills`, `mod overfilm_style`, `Session::overfilm_jobs`, `overfilm_caption_jobs`, `overfilm_assist_jobs` and `overfilm_kill_jobs`, their `poll` in `poll_persistence` |
| `crates/engine/src/commands.rs` | registers `overfilm_autocut::commands()`, `overfilm_captions::commands()`, `overfilm_transcribe::commands()`, `overfilm_assist::commands()`, `overfilm_reel::commands()`, `overfilm_kills::commands()` and `overfilm_style::commands()` |
| `crates/engine/src/transcript.rs` | `targets` is `pub(crate)` (which clips to recognise) |
| `crates/engine/src/overfilm_transcribe.rs` (+ `_tests.rs`) | new file: Korean captions in the editor |
| `crates/engine/src/overfilm_assist.rs` (+ `_tests.rs`) | new file: silence and highlight suggestions |
| `crates/engine/src/overfilm_reel.rs` (+ `_tests.rs`) | new file: the highlight reel |
| `crates/engine/src/overfilm_kills.rs` (+ `_tests.rs`) | new file: player bars, deaths and team fights |
| `crates/engine/src/overfilm_style.rs` (+ `_tests.rs`) | new file: the learnt editing style |
| `apps/filmcraft/Cargo.toml` | feature `overfilm-qwen`, on by default |
| `crates/ui-egui/src/panels/overfilm.rs` | new file: the OverFilm dialogs |
| `crates/ui-egui/src/panels.rs`, `menus.rs`, `panels/dialogs.rs` | `mod overfilm`, its `route` before the menu dialogs, its `show` |
| `crates/ui-egui/src/i18n/ko.tsv` | Korean labels of the OverFilm menu items and dialogs (appended) |
| `crates/engine/src/overfilm_autocut.rs` (+ `tests.rs`) | new file: Auto Cut commands (frames in, cuts out); `dissolve_at` is shared with the reel |
| `crates/engine/src/overfilm_captions.rs` | new file: styled captions and the Premiere Pro export |
