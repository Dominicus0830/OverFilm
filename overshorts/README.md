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

## 다음

- M2 LLM·대본, M3 음성, M4 렌더(Remotion), M5 패널
