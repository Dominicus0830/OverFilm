# Overwatch Korean pack

Data for Korean Overwatch captions. Milestone M2 (`OVERFILM.md`).

## `glossary/`

One TSV table per category, all with the columns described in `crates/overfilm-glossary`
(`id ko en aliases old misheard flags note`). Lists inside a field are `;`-separated.

| Table | Content |
|---|---|
| `heroes.tsv` | every hero with role and subrole, short forms, old names (`캐서디` ← `맥크리`) |
| `abilities.tsv` | weapons, abilities, passives and ultimates; ids are `<hero id>.<ability>` |
| `maps.tsv` | maps by mode, including the ones removed from regular play |
| `modes.tsv` | game modes and queues |
| `terms.tsv` | roles, subroles, mechanics, tactics, ranks, esports terms |
| `teams.tsv` | OWCS Korea teams (2026) and foreign teams they meet |
| `players.tsv` | OWCS Korea players and coaches (2026 Stage 2): Korean pronunciation in `ko`, handle in `en` |

- `ko` is how captions should write the term. For players it is the pronunciation commentators
  use (`D0NGHAK` → `동학`).
- `misheard` collects recognizer errors seen in the M1 test set (`로도그` for `로드호그`,
  `동아기` for `동학`). The M2 correction step rewrites them into `ko`.
- `flags`: `noeval` keeps a term out of term-accuracy scoring because it is an ordinary word too
  (`지원`, `부활`); `verify` marks a spelling not yet confirmed from an official Korean source.

Score with the whole glossary:

```sh
cargo run -p overfilm-eval -- --terms packs/overwatch/ko/glossary reference.srt hypothesis.srt
```

## Sources and state (2026-10-31)

- Heroes and subroles: Inven (2026-02-05) and Blizzard Korea's 2026 season posts; 시에라 (S2),
  시온 (S3), 디몬 (S4), 독트린 (S5) from the season announcements.
- New-hero ability names (도미나, 엠레, 미즈키, 안란, 제트팩 캣): Blizzard Korea's Season 1
  spotlight. 디몬, 독트린 and the Season 5 솜브라/로드호그 abilities are known only in English so
  far (`verify`, empty `ko`). Older heroes' ability names are the long-standing Korean client
  names, not re-checked against the current client; perks are not listed yet.
- Rosters: OWTV's OWCS Korea 2026 Stage 2 preview (June 2026). Stage 3 changes are not in yet.
  Pronunciations marked `발음 확인 필요` are guesses.
