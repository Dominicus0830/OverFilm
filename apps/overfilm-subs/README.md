# OverFilm 자막 추출 (overfilm-subs)

OverFilm에서 자막 추출만 떼어 낸 독립 프로그램입니다. 영상이나 오디오 파일을 넣으면 한국어 자막(SRT)을
만듭니다. 인터넷은 처음 모델을 받을 때만 쓰고, 소리는 이 컴퓨터 밖으로 나가지 않습니다.

- 음성 인식: Qwen3-ASR-1.7B (오버워치 영웅·용어 목록을 문맥으로 넣음), 단어 시간: Qwen3-ForcedAligner-0.6B.
  둘 다 Rust(candle)로 옮긴 것이라 Python이나 다른 프로그램이 필요 없습니다.
- 인식 뒤 단어집 교정(`packs/overwatch/ko/glossary`, 프로그램 안에 들어 있음): 자주 틀리는 표기, 이름 근접 오타,
  욕설 초성(씨발 → ㅅㅂ, 끌 수 있음).
- 맞춤법·띄어쓰기: 떨어진 조사 붙이기(이거 는 → 이거는), 의존 명사 띄우기(할수있다 → 할 수 있다, 하는거 → 하는 거),
  자주 틀리는 표기(할께요 → 할게요, 됬다 → 됐다, 그럴려면 → 그러려면, 벤 → 밴).
- 숫자: 말로 한 숫자를 자막식 숫자로(오 시즌 → 5시즌, 일 위 → 1위, 사십칠 퍼센트 → 47%, 여섯 개 → 6개). 뒤에
  단위가 올 때만 바꾸므로 '이 영웅', '이번 시즌'은 그대로입니다. 끌 수 있음(`--no-digits`).
- 자막: 최대 두 줄, 한 줄 16자, 쉼표·어미에서 자연스럽게 끊기.
- 결과: 입력 파일 옆에 `이름.srt`(UTF-8 BOM, Premiere에서 바로 가져오기)와 `이름.overfilm.json`(단어별 시간).

## 파일

| 파일 | 하는 일 |
|---|---|
| `OverFilmSubs.exe` | 창 프로그램. 더블클릭해서 엽니다. |
| `overfilm-subs.exe` | 명령줄과 MCP 서버. |

## 쓰는 법 (창)

1. `OverFilmSubs.exe`를 엽니다.
2. **영상/오디오 파일 열기**를 누르거나 파일을 창에 끌어다 놓습니다
   (mp4, mov, mkv, webm, avi, mxf, ts, mp3, m4a, aac, wav, flac, ogg, opus).
3. 영상 종류를 고릅니다: 일반 게임 영상, 또는 대회 해설(팀·선수 이름을 문맥에 더함).
   스트리머 이름처럼 단어집에 없는 이름은 **추가 이름·용어**에 쉼표로 적습니다.
4. **자막 만들기**. 처음 한 번은 모델(약 6.5GB)을 받습니다.
5. 끝나면 미리보기와 **검토할 곳**(단어집과 다른 표기, 읽기 속도, 겹침 등)이 나옵니다.

모델 위치: `%LOCALAPPDATA%\OverFilm\models`. 프로그램 옆에 `models` 폴더가 있으면 그것을 씁니다
(USB에 통째로 들고 다닐 때). 다른 곳을 쓰려면 환경 변수 `OVERFILM_MODELS`.

## 명령줄

```bat
overfilm-subs models
overfilm-subs extract "경기.mp4" --mode commentary --terms "새 선수, 새 팀"
overfilm-subs extract "방송.m4a" -o 자막.srt --no-mask --line-chars 18
overfilm-subs review 자막.srt          검토할 곳 목록
overfilm-subs fix 자막.srt             단어집 교정 적용 (원본은 자막.srt.bak)
overfilm-subs relayout 경기.overfilm.json --line-chars 18   인식을 다시 하지 않고 자막만 다시 나누기
```

## MCP로 검토하기 (Claude Desktop)

창의 **MCP 연결** 버튼이 아래 설정을 경로까지 채워서 보여 주고 복사해 줍니다. Claude Desktop → 설정 → 개발자 →
설정 편집(`claude_desktop_config.json`)에 넣고 Claude Desktop을 다시 시작합니다.

```json
{
  "mcpServers": {
    "overfilm-subs": { "command": "C:\\OverFilm\\overfilm-subs.exe", "args": ["mcp"] }
  }
}
```

대화에서 할 수 있는 일:

| 도구 | 하는 일 |
|---|---|
| `extract_subtitles`, `job_status`, `cancel_job` | 자막 추출 (백그라운드, 진행률 확인) |
| `read_subtitles` | 자막 읽기 |
| `review_subtitles` | 검토: 단어집·맞춤법과 다른 표기(고칠 글자 제안), 읽기 속도, 줄 길이·줄 수, 너무 짧음, 겹침, 반복, 빈 자막 |
| `find_silences` | 말 없는 구간 표시 (지우지 않음) |
| `edit_subtitle`, `split_subtitle`, `merge_subtitles`, `delete_subtitle` | 고치기 (처음 고칠 때 `.srt.bak` 백업) |
| `apply_glossary` | 단어집·맞춤법·숫자 교정 적용 |
| `relayout_subtitles` | 줄바꿈 다시 하기 |
| `add_glossary_term` | 자주 틀리는 표기를 사용자 단어집(`%LOCALAPPDATA%\OverFilm\glossary\user.tsv`)에 추가 |

예: "경기.mp4 자막 뽑고, 끝나면 검토해서 선수 이름 틀린 것 고쳐 줘."

## 속도와 사양

- 그래픽카드(GPU)가 있으면 씁니다: AMD 라데온, NVIDIA, 인텔 모두 Windows의 Vulkan이나 DX12로 돌아가고 따로 설치할 것은 없습니다. 그래픽 메모리는 약 4GB를 씁니다. 화면의 '그래픽카드(GPU) 사용'을 끄거나 `--cpu`를 주면 CPU만 씁니다.
- GPU가 없거나 GPU에서 오류가 나면 자동으로 CPU로 이어서 처리합니다. 완료 메시지에 어느 쪽에서 돌았는지 나옵니다.
- CPU만 쓸 때는 AVX2가 있는 x86-64 CPU(2013년 이후 인텔·AMD)가 필요하고 메모리는 약 4GB를 씁니다. 4코어 기준 소리 1분에 약 2분 40초 걸립니다 (모델 불러오기 포함, 2분짜리 영상 실측 5분 15초).

## 빌드

```sh
# Windows에서
cargo build --release -p overfilm-subs
# 리눅스에서 Windows용 (mingw-w64 필요)
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-C target-cpu=x86-64-v3" \
cargo build --release --target x86_64-pc-windows-gnu -p overfilm-subs
```

`target-cpu=x86-64-v3`(AVX2)가 없으면 8비트 계산이 SIMD를 못 써서 몇 배 느려집니다.
