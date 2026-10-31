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
- 목소리 구별: 말하는 사람이 몇 명인지 세고, 사람이 둘 이상이면 자막마다 `(화자1)`, `(화자2)`처럼 누가 말했는지
  붙입니다. 자막 하나에 두 사람 말이 섞이지 않습니다. 사람 수를 알면 정해 줄 수 있고(`--speakers 3`), 이름은 나중에
  바꿀 수 있습니다(`relayout … --rename 화자1=아이치`, MCP `rename_speaker`). 사람마다 SRT를 따로 저장할 수도 있습니다
  (`--per-speaker`, Premiere에서 해설자마다 자막 트랙을 따로 둘 때).
- 목소리 등록: 스트리머·해설자처럼 자주 나오는 사람의 목소리를 한 번 등록해 두면(그 사람만 말하는 10~30초), 다음
  영상부터 `화자1` 대신 그 이름이 붙고, **등록한 목소리만 자막**을 켜면 그 사람 말만 자막으로 만듭니다(게임 음성·다른
  사람 제외). `overfilm-subs voice add 이름 영상.mp4 --from 1:23 --to 1:45`, 창의 **목소리 등록…**, MCP `add_voice`.
- 게임 음성: 영웅 궁극기 대사·아나운서 같은 게임 소리를 찾아 자막에서 뺍니다(기본). `(게임)`으로 표시하거나 그대로 둘 수도
  있습니다(`--game-voice label|keep`). 자세한 방식과 한계는 아래 "목소리 구별과 게임 음성".
- 결과: 입력 파일 옆에 `이름.srt`(UTF-8 BOM, Premiere에서 바로 가져오기)와 `이름.overfilm.json`(단어별 시간과 화자).

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
   여러 사람이 말하는 영상이면 **목소리 구별**을 켜 둡니다(기본으로 켜짐). 사람 수를 알면 정해 주세요.
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
overfilm-subs extract "합방.mp4" --speakers 3 --per-speaker     사람 3명, 사람마다 SRT도 저장
overfilm-subs extract "방송.mp4" --game-voice label             게임 음성을 빼지 않고 (게임)으로 표시
overfilm-subs relayout 합방.overfilm.json --rename 화자1=아이치 --rename 화자2=게스트   화자 이름 바꾸기
overfilm-subs voice add 아이치 "방송.mp4" --from 1:23 --to 1:45   목소리 등록 (그 사람만 말하는 구간)
overfilm-subs extract "방송.mp4" --only-voices                    등록한 목소리(스트리머)의 말만 자막
overfilm-subs voice list
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
| `add_voice`, `list_voices`, `remove_voice` | 목소리 등록·목록·삭제 (`extract_subtitles`의 `only_voices`로 그 사람 말만 자막) |
| `list_speakers`, `rename_speaker` | 구별된 화자 목록(말한 시간, 게임 음성 여부), 이름 바꾸기·게임 음성으로 돌리기 (자막 다시 만듦) |
| `add_glossary_term` | 자주 틀리는 표기를 사용자 단어집(`%LOCALAPPDATA%\OverFilm\glossary\user.tsv`)에 추가 |

예: "경기.mp4 자막 뽑고, 끝나면 검토해서 선수 이름 틀린 것 고쳐 줘."

## 목소리 구별과 게임 음성

- **어떻게**: 인식된 단어가 있는 곳을 1.5초 창으로 0.5초마다 훑으며 목소리 특징(256개 숫자)을 뽑고, 비슷한 것끼리
  묶어(평균 연결 군집, 코사인 거리 0.7에서 멈춤) 사람 수를 정합니다. 말이 10초(짧은 영상은 전체의 8%)가 안 되는
  목소리는 가장 비슷한 사람에게 합칩니다. 단어마다 각 사람과 얼마나 비슷한지 점수를 매기고, 사람이 바뀌는
  곳은 쉼(0.4초 이상)이나 문장 끝에서만 쉽게 바뀌도록 정합니다. 그래서 한 문장이 중간에 다른 사람으로 쪼개지지 않습니다.
- **모델**: WeSpeaker ResNet34 (VoxCeleb으로 학습, pyannote 배포본, CC-BY-4.0, 27MB)와 겹친 말용 pyannote segmentation 3.0
  (MIT, 6MB). 처음 한 번 받습니다.
  Rust(candle)로 옮겼고 원본(ONNX)과 결과가 같습니다(코사인 1.0000).
- **정확도**: 한국어 낭독 음성(Zeroth, 10명)으로 만든 2~5명 대화 24개(각 2분 30초, 짧은 맞장구 포함)에서 사람 수
  24/24 정답, 단어의 화자 오류 1.9%. 실제 방송은 말이 겹치고 게임 소리가 섞여 이보다 어렵습니다.
- **게임 음성**: `packs/overwatch/ko/voicelines.tsv`(약 4100줄: 대부분 영웅의 궁극기 대사·아나운서, 그리고 겐지·아나·키리코·메르시·라인하르트·솔저: 76의
  한국어 대사 전부)와 글자로 맞춰 봅니다. "방어구가 필요해" 같은 소통 대사는 사람도 하는 말이라 그것만으로는 빼지 않습니다. 다섯 글자
  이상인 대사와 같으면(여덟 글자 이상은 한두 글자 달라도) 게임 음성으로 보고, 한 목소리의 말이 대부분 대사이면 그 목소리 전체를 '게임 음성'으로
  봅니다. 나머지 영웅의 일반 대사(처치·리스폰·경기 시작 대화 등)는 아직 목록에 없어 못 찾고, 스트리머가 대사를
  따라 하면 같이 빠질 수 있습니다. 잘못 빠졌으면 `--game-voice label`로 다시 만들어 확인하세요.
- **목소리 등록**: 샘플의 목소리 특징 평균을 `%LOCALAPPDATA%\OverFilm\voices\이름.json`에 저장합니다(소리는
  저장하지 않음). 영상에서 구별된 사람 중 등록한 목소리와 충분히 가까운(코사인 거리 0.55 이하) 사람에게 그 이름을
  붙이고, 가장 가까운 짝부터 하나씩만 붙입니다. 등록한 사람은 말이 적어도 따로 남깁니다.
  합성 대화 24개로 시험하면 단어의 97.9%에 맞는 이름이 붙고, 틀린 이름은 2.1%입니다.
- **겹친 말**: 두 사람이 동시에 말하는 곳을 pyannote segmentation 3.0(MIT, 6MB, 로그인 없이 받는 공개 사본)으로 찾아
  단어마다 '같이 말한 사람'을 붙입니다. 두 단어 이상, 1초 이상 이어지면 자막에 `(화자1·화자2)`로 표시하고, 사람별
  자막(`--per-speaker`)과 등록한 목소리만 자막(`--only-voices`)에는 다른 사람 말 위에 겹쳐 한 말도 넣습니다. 게임 음성
  위에 사람이 말한 단어(대사가 아닌 것)는 사람 말로 돌립니다. 합성 대화(실제로 두 목소리가 들리는 단어 301개)에서 겹친 단어의 75%를 찾고
  64%는 두 사람 모두 맞혔으며, 겹치지 않은 단어를 겹쳤다고 한 경우는 2.8%입니다. 인식기는 한 줄로만 받아 적으므로
  겹친 구간의 어느 단어를 누가 했는지까지는 나누지 못합니다. 4코어 CPU에서 소리 10분에 약 1분 더 걸립니다.
- **한계**: 겹친 말의 4분의 1 정도는 아직 놓칩니다(주로 한쪽 목소리가 작을 때). 목소리가 아주 비슷하거나 한 사람이 목소리를 크게 바꾸면
  틀릴 수 있습니다. 틀린 이름은 `rename_speaker`로 고치고, 사람 수를 알면 정해 주는 것이 가장 확실합니다.

## 속도와 사양

- 그래픽카드(GPU)가 있으면 씁니다: AMD 라데온, NVIDIA, 인텔 모두 Windows의 Vulkan이나 DX12로 돌아가고 따로 설치할 것은 없습니다. 그래픽 메모리는 약 4GB를 씁니다. 화면의 '그래픽카드(GPU) 사용'을 끄거나 `--cpu`를 주면 CPU만 씁니다.
- GPU가 없거나 GPU에서 오류가 나면 자동으로 CPU로 이어서 처리합니다. 완료 메시지에 어느 쪽에서 돌았는지 나옵니다.
- CPU만 쓸 때는 AVX2가 있는 x86-64 CPU(2013년 이후 인텔·AMD)가 필요하고 메모리는 약 4GB를 씁니다. 4코어 기준 소리 1분에 약 2분 40초 걸립니다 (모델 불러오기 포함, 2분짜리 영상 실측 5분 15초).

- 목소리 구별도 그래픽카드에서 돕니다(시작할 때 CPU와 결과를 비교해 다르면 CPU로). CPU만 쓰면 소리 10분에 1~2분 더
  걸립니다(4코어 실측: 20초에 2.3초). 끄면(`--speakers off`) 이 시간이 빠집니다.
  10분짜리 해설 영상 전체(인식 + 목소리 구별)를 4코어 CPU만으로 처리하는 데 32분 걸렸습니다.

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
