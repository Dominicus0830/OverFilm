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
| `players.tsv` | OWCS Korea players and coaches (2026 Stage 3), then earlier players: Korean pronunciation in `ko`, handle in `en`, real name in `note` |
| `slang.tsv` | community and broadcast slang: role and comp names (`멘딜`, `정파탱`), tactics (`앵커링`, `딜각`), memes (`딸깍`, `C9`), platforms, comp and duo names (`메모위`, `고츠`, `볼토시`, `투스나`, `파르시`, `자자`), `차이` words (`딜러차이`), callouts (`딸피`, `합궁`, `역궁`) |
| `netslang.tsv` | everyday internet and streaming slang that streamers say out loud: abbreviations (`알잘딱깔센`, `네카오`, `중꺾마`), reactions (`킹받네`, `억까`, `뇌절`), broadcast words (`뱅온`, `방종`, `합방`, `도네`), gallery words (`개추`, `실베`); spaced spellings in `misheard` (`알 잘 딱` → `알잘딱깔센`), slurs left out |
| `profanity.tsv` | swear words: captions write only the initial consonants (`씨발` → `ㅅㅂ`, `존나` → `ㅈㄴ`); spoken forms are in `misheard`, flag `mask` |
| `spelling.tsv` | spelling fixes that are not terms: `벤` for `밴` (ban: `강벤` → `강밴`, `벤률` → `밴률`), tier words written apart (`저 티어` → `저티어`), statistics words (`괴류율` → `괴리율`), and common misspellings (`몇일` → `며칠`, `어의없` → `어이없`); all `noeval`, `inword` for forms that are wrong inside a word too (`뵈요` → `봬요`). Grammar rules that are not single words (particles, `할 수 있다`, `할게요`) are code: `overfilm-glossary` `ko` module |
| `matchups.tsv` | team-vs-team short names: first syllables of both teams + `전` (`라제전` = 크레이지 라쿤 vs 제타, also `제라전`; 치즈버거 as 치 and 처 from 치벅/처벅) |

- `ko` is how captions should write the term. For players it is the pronunciation commentators
  use (`D0NGHAK` → `동학`).
- `misheard` collects recognizer errors seen in the M1 test set (`로도그` for `로드호그`,
  `동아기` for `동학`). The M2 correction step rewrites them into `ko`.
- `flags`: `noeval` keeps a term out of term-accuracy scoring because it is an ordinary word too
  (`지원`, `부활`); `verify` marks a spelling not yet confirmed from an official Korean source;
  `mask` marks profanity, which the correction step replaces with `ko` even inside a word
  (`씨발아` → `ㅅㅂ아`), longest form first; `inword` makes an ordinary `misheard` form match inside
  a word too.
- A `misheard` form ending in `@픽` is an ordinary word that sounds like a hero (`캐슬` for `캣`): it is
  replaced only when a hero-pick word follows (`쓰`, `픽`, `꺼내`, `하면`…), so `캐슬 쓰게 되면` → `캣 쓰게 되면`
  but `모래 캐슬` stays.
- Before `선수`, `감독`, `코치`, player names are matched more loosely: two-syllable names too, one more
  edit, `ㅔ`/`ㅐ` alike and a doubled consonant once (`백기 선수` → `베키 선수`).

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
  reported by the user follow the same rule and are marked `verify`. Slurs are not collected; common swear words are, only so captions can mask them
  (`profanity.tsv`).
- 2026-11-01 additions: 나무위키 (namu.moe) `오버워치/용어`, `오버워치/조합`, `고츠 조합`, `메모위주`,
  `차이(게임 은어)`, `신조어`, `알잘딱깔센`, `중요한 것은 꺾이지 않는 마음`; 더위키 `SOOP/방송 용어`; 우리말샘 glosses
  (via yugacrew); DCinside 오버워치2 and OWCS gallery boards. Comp names the pages only name (`겐메`, `윈솜트`) are `verify`.
- The slang is used for correction and to keep the name fixer off these words, not as recognition
  context: adding 25 of them (`메모위`, `고츠`, `알잘딱깔센`, `뱅온`…) to `context.txt` took the M1 aichi
  video from 6.14% to 6.29% CER and its term score from 99.1% to 95.5% (`뒷라인` → `뒤라인`,
  `메인딜러` → `메인 딜러`), so they stay out. A video full of one slang word can take it through
  `--terms`.
- Overseas teams and Korean players and coaches abroad (`players.tsv` bottom, `teams.tsv` bottom): owtics.gg team
  pages (2026-11-01). Readings from 나무위키 where it gives one (페이즈/프사제, 크리링, 스큐드, 프로핏); the rest
  are read as spelled and marked `발음 확인 필요`. Misrecognitions from the T1 vs RØDE commentary clip
  (`티온` → `티원`, `동커퀸` → `정커퀸`, `아이언발드` → `아이헨발데`, `델로스피얼` → `댈러스 퓨얼`).
- 2026-11-02: ability and perk names from Blizzard's Korean hero pages (overwatch.blizzard.com/ko-kr/heroes,
  54 heroes): the 디몬, 독트린, 솜브라, 로드호그 and other `verify` names filled in, 27 names corrected to the
  client's (`광란` → `살육`, `표창` → `수리검`, `순보` → `이단뛰기`; the earlier spelling is kept in `old`), 69
  abilities added and 216 perks (`noeval`). 30 more regional teams from owtics.gg (`verify` readings),
  and tech words (`캔슬`, `노줌`, `지뢰 점프`, `갈고리 스윙`) and match words (`팟지`, `킬딸`, `막거점`) in
  `slang.tsv`.
- 2026-11-02 user report: commentary words that come out wrong (`궁극기` heard as `공격기`, `렐리` → `랠리`,
  `잭팩켓` → `제트팩 캣`, `자랴` → `자리야`, `오투 블러스터` → `오투 블래스트`), hero short names (`라위`, `바스`,
  `트레`), team names checked on 나무위키 (`로데 잔사이드 게이밍`, `오투 블래스트`) and play verbs
  (`물다`, `눕다`, `끊다`, `녹이다`, `궁 빼다`, `비비다`, `밀다`; `noeval`). `눕다` means dying, so it moved
  off `앵커링`. Floors: `이층` → `2층`; `이 층` stays unless next to another floor number.
- Team short names from the DCinside OWCS gallery: 라쿤, 제타, 팔콘, 티원, 로제, O2, 치벅/처벅/CB,
  세이지 (SEIJI ESPORTS).
- Rosters: 나무위키 (namu.moe mirror) team and player pages, 2026 OWCS Korea Stage 3
  (2026-10-02 – 11-08; nine teams, SuperBad disbanded 2026-08-31), read 2026-10-31. Korean
  readings come from 나무위키 where it gives one (스핀트, 헤이저, 메리트, 미닛), from broadcasts and
  the galleries otherwise; the rest are marked `발음 확인 필요`.
