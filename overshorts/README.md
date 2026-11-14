# OverShorts (overfilm-shorts)

DC 갤러리 링크나 게시물 링크를 넣으면 원문을 각색한 30~60초 세로 쇼츠를 만들어 검수 큐에 올리는
로컬 프로그램이다. 기획서: 2026-11-15 「OverShorts 기획서 — DC 갤러리 → 쇼츠 자동화」.

OverFilm 본체(Rust)와 따로 도는 프로그램이라 Rust 워크스페이스 밖 이 폴더에 둔다.
기획서의 결정대로 백엔드는 Python, 렌더(`render/`)와 패널(`panel/`)은 TypeScript로 붙일 예정이다.

## 지금 되는 것 (M1 수집)

| 기능 | 파일 |
| --- | --- |
| 링크 자동 인식: 정식·마이너·미니 갤 목록, 게시물, 모바일, 개념글 | `core/overshorts/urls.py` |
| 목록·본문·댓글 파서 (셀렉터는 이 파일에만) | `core/overshorts/crawler/dc_parser.py` |
| 요청 예산(시간 60·하루 200), 1.5~4초 랜덤 간격, 글 10개마다 20~40초 휴식 | `core/overshorts/crawler/budget.py` |
| 차단 감지: 403·429·캡차·리디렉션 반복 → 즉시 중단, 60분 쿨다운, 또 막히면 그날 종료. 느려지면 간격 2배 | `core/overshorts/crawler/budget.py` |
| curl_cffi(크롬 흉내) 요청, 한 번에 하나만 | `core/overshorts/crawler/fetcher.py` |
| 목록의 추천·날짜로 먼저 거르고 후보만 본문 요청, 받은 글·폐기한 글은 다시 안 받음, 원본 HTML 캐시 | `core/overshorts/crawler/collector.py` |
| SQLite (sources, posts, filter_results, scripts, voices, jobs + 요청 기록·쿨다운 상태) | `core/overshorts/db.py` |
| 1차 규칙 필터: 전화번호·이메일·주민번호·계좌번호, 짧은/긴 글, 그림만 있는 글, 금지어 | `core/overshorts/filters/rules.py` |
| 첨부 그림 받기(`Referer` 헤더, 글당 5장, 예산에 같이 셈) | `core/overshorts/crawler/collector.py` |
| 그림 검사: 얼굴 흐림(OpenCV YuNet, MIT), 선정성 분류(Falconsai ViT ONNX, Apache-2.0)에 걸리면 뺌, 검사 못 한 그림은 검수 표시 | `core/overshorts/images.py` |

## M2 LLM·대본

| 기능 | 파일 |
| --- | --- |
| LLM 어댑터: `claude -p`(빈 임시 폴더, 도구 끔, API 키 지움) + Ollama(JSON 스키마), pydantic 검증·2번 재질문, 한도 시 로컬로 넘김 | `core/overshorts/llm/` |
| LLM 필터: 정치·혐오·성적·실존 인물 비방·개인정보·범죄·광고 → pass/block/review + 적합도 | `core/overshorts/filters/llm_filter.py` |
| 대본 프롬프트: 훅·전개·댓글·마무리, 톤 3종, 반말/해요체, 스타일 파일 반영 | `core/overshorts/script/prompts.py` |
| 규칙 검사: 금지 표현, ~노 어미, 전언 반복, 긴 문장·단조로운 길이, 지어낸 댓글, 원문 겹침(글자 5-gram), 예상 길이 | `core/overshorts/script/checks.py` |
| 오글 검사(별도 호출 0~10), 기준 미만·규칙 위반이면 사유 붙여 다시 씀(최대 2번), 남은 문제는 검수 경고 | `core/overshorts/script/writer.py` |
| 스타일 파일: `style/guide.md`, `banned.txt`, `good/`, `bad/` | `style/` |

게시물 본문·댓글은 `<post>`·`<comments>` 태그로 감싸고 안의 지시를 따르지 말라고 프롬프트에 넣는다.
2026-11-14 실제 `claude -p`(sonnet)로 테스트 글 대본: 30초 안팎, 원문 겹침 10%, 오글 7.5점, 다시 쓰기 1번.

## M3 음성

| 기능 | 파일 |
| --- | --- |
| Edge-TTS 엔진(단어 경계 시각), `synthesize(text, voice)` 하나로 맞춤 | `core/overshorts/tts/engines.py` |
| 문장 단위 합성 + 0.15초 간격, 댓글 줄은 댓글 보이스, 따옴표 등은 읽지 않음, FFmpeg 로 48kHz 모노 wav | `core/overshorts/tts/assemble.py` |
| 단어 시각 없으면 음절 수 비율로 나눔(정렬 실패 대비), `timing.json` 에 문장·단어 시각 | `core/overshorts/tts/assemble.py` |
| 작업 단계: scripted → aligned, `data/jobs/<id>/voice.wav`, `timing.json` | `core/overshorts/tts/stage.py` |

2026-11-14 실측: 대본 6문장 → 23.3초 wav, 합성 6초, 단어 시각은 숫자 원문 그대로(`500힐밖에`).
Edge-TTS 는 비공식 엔드포인트라 수익화 채널은 유료 API 권장 (기획서 5.3).

## M4 렌더

| 기능 | 파일 |
| --- | --- |
| `render_spec.json` 만들기: 장면·단어 자막·핵심 단어 강조(숫자, 제목 낱말 최대 6개)·배경·출처 | `core/overshorts/render/spec.py` |
| Remotion `CardStory`: 상단 제목바, 중앙 글 카드(읽는 단어까지 밝게, 댓글은 파란 카드), 단어 팝업 자막, 배경 영상 반복, 출처 | `render/src/CardStory.tsx` |
| 렌더 실행: 승인된 작업만, GPU 감지 → NVIDIA 는 NVENC, 라데온·인텔은 CPU x264, Windows 는 `--gl=angle` | `core/overshorts/render/runner.py` |

글꼴 Pretendard(OFL)는 저장소에 넣지 않고 렌더할 때 CDN 에서 불러온다(오프라인이면 맑은 고딕).
배경 영상은 직접 녹화한 것이나 라이선스 있는 것만 `render.background_dir` 에 넣는다.
2026-11-14 클라우드 CPU 시험 렌더: 24초 1080×1920 60fps, 1분 44초, 3.2MB (공유 폴더 `overshorts/CardStory-시험-렌더.mp4`).
라데온 AMF 하드웨어 인코딩은 아직 없음: Remotion 번들 FFmpeg 은 Windows 에서 NVENC 만 지원.

## M5 패널 핵심

`overshorts serve` 로 띄우면 브라우저가 열린다 (`http://127.0.0.1:8765/`, 이 PC 에서만 접속).

| 화면 | 하는 일 |
| --- | --- |
| 홈 | 오늘 수집·생성·승인 수, 크롤링 예산 게이지·차단 상태, Claude 한도, GPU, 다음 할 일 |
| 새로 만들기 | 링크 입력 → 종류 배지 자동 표시, 갤러리 모드 경고, 톤, ‘고급’에 개념글·추천·기간·개수 → 시작 |
| 작업 큐 | 단계별 진행률, 실패 사유와 ‘이 단계부터 다시’, 일시정지, 실시간 로그(WebSocket) |
| 검수 | 와이어프레임대로 원문(대본과 겹치는 글자 노란 표시) / 대본(문장별 편집, ▶ 그 문장만 듣기, 오글? 표시, 오글 신고) / 9:16 Player 미리보기. 승인 A·수정 반영 E·재생성 R(톤→대본부터, 프리셋→렌더 설정만)·폐기 D, J/K 이동 |

보이스·스타일·소스·설정 화면은 M8. 오글 신고는 `style/reports.jsonl` 에 쌓이고 M8 스타일 화면에서 금지 목록 반영을 승인한다.
긴 작업은 작업 스레드 하나가 차례로 돈다 (크롤링은 한 줄로만, GPU 모델은 한 번에 하나만).

```sh
cd overshorts/panel && npm install && npm run build      # 패널 화면 (한 번만)
cd ../render && npm install                              # 렌더 템플릿 (한 번만)
cd ../core && pip install -e .[dev,images,panel] && overshorts serve
```

2026-11-14 실측 (요청 4번): 목록 `tbody tr.us-post`, 본문 `div.write_div`, 공지·AD·설문 행 구분 확인.
댓글은 HTML에 없고 `POST https://gall.dcinside.com/board/comment/` 가 JSON으로 준다. 브라우저 없이
받아지므로 Playwright 폴백은 아직 넣지 않았다. 정상 글 페이지에도 추천용 `kcaptcha` 가 있어서,
캡차 판정은 정상 내용이 없는 페이지에서만 한다.

## 실행

```sh
cd overshorts/core
python -m venv .venv && .venv/Scripts/activate      # macOS·Linux: source .venv/bin/activate
pip install -e .[dev,images]
copy ..\config.example.yaml config.yaml              # 선택

overshorts parse "https://m.dcinside.com/board/overwatch2/1981822"   # 요청 없이 종류만
overshorts ingest "https://gall.dcinside.com/mgallery/board/lists/?id=overwatch2" --recommend-only --min-upvotes 30 --max-posts 5
overshorts inspect                                                    # 받은 그림 검사 (첫 실행 때 모델 약 90MB 받음)
overshorts filter                                                     # LLM 필터 (Ollama 켜 두기)
overshorts script                                                     # 대본 (Claude Code 로그인 필요)
overshorts voice                                                      # 음성 + 단어 시각 (FFmpeg 필요)
overshorts preview                                                    # 미리보기 spec
overshorts approve 3                                                  # 승인 (패널 전 임시)
overshorts render                                                     # 최종 MP4 (render 폴더에서 npm install 먼저)
overshorts budget                                                     # 오늘 예산·쿨다운
pytest                                                                # 저장 HTML 테스트 (네트워크 안 씀)
```

갤러리 전체를 자주 긁지 말 것. 기본 예산은 보고된 차단 기준(단일 IP 약 240건 지연, 620건 차단)보다
훨씬 낮게 잡혀 있고, 설정에서 올리면 경고가 뜬다.

## GPU (사용자 결정 2026-11-14)

기획서는 NVIDIA 전용이었지만 라데온(RX 9060 XT)도 지원하도록 바꾼다. LLM 필터는 Ollama의 AMD 지원,
인코딩은 `encoder: auto`(NVIDIA → NVENC, AMD → AMF/미디어 파운데이션, 없으면 CPU), 클로닝은 CUDA가
없으면 CPU로 느리게 허용한다. 그림 검사는 onnxruntime DirectML 로 어느 GPU에서나 돈다.
렌더(M4)·클로닝(M6)·`doctor`(M7)에서 반영한다.

## 기획서 갱신 반영 (2026-11-15, 개정 5·6)

| 바뀐 것 | 위치 |
|---|---|
| 프레임형 기본 프리셋: 검은 배경, 제목 15~26% 고정, 소제목 33~38%, 원문 미디어 39~66%, 노란 2~4단어 자막 67~72%, 78% 아래 비움. 원문 미디어 없으면 썰 카드형 자동 | `render/src/FrameStory.tsx`, `Story.tsx` |
| 디시 스킨 본문 글 카드·댓글창 (닉네임 열·IP 앞자리·고닉 아이콘, 답글 └ 들여쓰기, @멘션 파란 글자, 디시콘+‘디시콘 보기’, 하나씩 쌓이고 넘치면 스크롤, 읽는 댓글 강조 테두리). 닉네임 표시 설정 original/blur/replace | `render/src/DcSkin.tsx` |
| 세그먼트 대본: AI가 구성 A(제목→본문→댓글)/B(제목→본문), 세그먼트마다 TTS 켜기/끄기와 이유, 미디어를 어느 문장에 어떤 표시 방식(fill/crop/full/tall/scroll)으로 둘지 고름 | `core/overshorts/script/segments.py`, `models.py`, `prompts.py` |
| 댓글은 만들지 않음: 번호로만 고르고 화면·음성은 원문 그대로. 답글은 부모와 같이, 욕설·혐오 댓글은 경고 | `segments.validate` |
| TTS 끈 세그먼트는 무음으로 화면 시간 확보(pause 줄), 검수에서 세그먼트별 TTS 바꾸기 | `tts/assemble.py`, 패널 검수 |
| 무음 5초 넘으면 BGM(`render.bgm_dir`) 깔기, 말할 때 −12dB 덕킹. BGM 없으면 검수 경고 | `render/spec.py`, `FrameStory.tsx` |
| 수집: 댓글 답글 관계·닉네임·IP·고닉 아이콘·디시콘(그림 최대 8개) 저장, 본문 속 동영상(직접 받을 수 있는 mp4·webm, 최대 3개), 본문에 `[사진 N]` 자리 표시, 사진 6장 | `crawler/` |
| 확인창: 갤러리 크롤링(예상 요청 수·남은 예산, ‘계속’), Claude 한도 → 로컬 LLM 전환/대기, 둘 다 ‘다시 묻지 않기’ | `server/app.py`, 패널 |

아직 없음: 원문 미디어 분석(VLM·OCR 닉네임 흐림·장면 분할·물체 따라 크롭)은 표시 방식 이름만 받고
크롭은 가운데 기준, 동영상 프레임 검사, Playwright 실제 댓글 캡처 모드, M9 양산(배치·변주·루프·효과음·메타데이터·썸네일·플랫폼).

## 다음

- M6 클로닝(GPT-SoVITS, AMD 는 CPU), M7 OverFilm MCP·`doctor`, M8 설정 화면 전부
