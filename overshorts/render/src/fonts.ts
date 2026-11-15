// Pretendard, Black Han Sans, Gaegu (모두 SIL OFL 1.1). 글꼴 파일은 저장소에 넣지 않고 렌더할 때 CDN 에서 불러온다.
// Black Han Sans·Gaegu 는 짤 장면 대사 자막(버럭·시무룩 등)에만 쓴다.
import {continueRender, delayRender} from 'remotion';

const CSS = 'https://cdn.jsdelivr.net/gh/orioncactus/pretendard@v1.3.9/dist/web/static/pretendard.min.css';
const EXTRA = 'https://fonts.googleapis.com/css2?family=Black+Han+Sans&family=Gaegu:wght@700&display=block';
export const SHOUT_FONT = "'Black Han Sans', " + "Pretendard, 'Malgun Gothic', sans-serif";
export const HAND_FONT = "Gaegu, Pretendard, 'Malgun Gothic', sans-serif";
let loaded = false;

export const FONT_FAMILY =
  "Pretendard, 'Apple SD Gothic Neo', 'Malgun Gothic', 'Noto Sans KR', sans-serif";

export function loadFonts(): void {
  if (loaded || typeof document === 'undefined') return;
  loaded = true;
  for (const href of [CSS, EXTRA]) {
    const handle = delayRender(`글꼴 불러오기 ${href}`);
    const link = document.createElement('link');
    link.rel = 'stylesheet';
    link.href = href;
    const done = () => {
      // 쓰는 글자를 미리 불러와 첫 프레임부터 같은 글꼴로
      Promise.all([document.fonts.load("40px 'Black Han Sans'", '가'), document.fonts.load('40px Gaegu', '가')])
        .then(() => document.fonts.ready).then(() => continueRender(handle)).catch(() => continueRender(handle));
    };
    link.onload = done;
    link.onerror = () => continueRender(handle); // 오프라인이면 시스템 글꼴로
    document.head.appendChild(link);
  }
}
