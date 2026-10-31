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
| `overfilm-glossary` | L1 | M2 | glossary tables (shared term ids, spellings, old names, misrecognitions) and their term-list export; transcript correction next |
| `overfilm-asr` | L2 | M3, M5 | Qwen3-ASR + Qwen3-ForcedAligner ported to candle as a `filmcraft_speech::Transcriber`; streamer/hero-voice separation |
| `overfilm-captions` | L2 | M4 | Korean caption layout: two lines at most, display-width limits, penalty-based breaks |
| `overfilm-eval` | L3 | M0 | transcript evaluation: CER, WER, game-term accuracy (library + `overfilm-eval` binary) |
| `overfilm-ai` | L4 | M6, M7, M9 | MCP editing assistants, screen recognition, style learning |
| `packs/overwatch/ko/` | data | M2 | Overwatch Korean pack: `glossary/` (heroes, abilities, maps, modes, terms, teams, players; see its README), later hero voice lines, fillers, caption rules |

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
  profanity to initial consonants), to measure what the correction step adds.
  Matching ignores spacing and punctuation and takes the longest term at each position.
- Both sides are normalized the same way: caption markup removed, decomposed Hangul composed,
  full-width ASCII folded, digits read out in Korean (`6개` = `여섯 개`, `1위` = `일 위`),
  lowercase, punctuation as spaces. Corpus totals sum edits over files.

`crates/overfilm-eval/samples/` has a hand-counted example; its README lists the expected numbers.

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
