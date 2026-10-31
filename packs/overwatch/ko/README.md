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
| `slang.tsv` | community and broadcast slang: role and comp names (`멘딜`, `정파탱`), tactics (`앵커링`, `딜각`), memes (`딸깍`, `C9`), platforms |
| `matchups.tsv` | team-vs-team short names: first syllables of both teams + `전` (`라제전` = 크레이지 라쿤 vs 제타, also `제라전`) |

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
- Nicknames and slang: 나무위키 and 더위키 `오버워치/용어` (2026), DCinside 오버워치 갤러리 and
  오버워치2 마이너 갤러리 post titles (2026-10-31). Hero and ability nicknames are in `aliases`
  (`원숭이` 윈스턴, `석양` 황야의 무법자, `뽕` 나노 강화제). Matchup names other than the ones
  reported by the user follow the same rule and are marked `verify`. Profanity and slurs are not
  collected.
- Rosters: OWTV's OWCS Korea 2026 Stage 2 preview (June 2026). Stage 3 changes are not in yet.
  Pronunciations marked `발음 확인 필요` are guesses.
