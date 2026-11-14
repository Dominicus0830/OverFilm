// Pretendard (SIL OFL 1.1). 글꼴 파일은 저장소에 넣지 않고 렌더할 때 CDN 에서 불러온다.
import {continueRender, delayRender} from 'remotion';

const CSS = 'https://cdn.jsdelivr.net/gh/orioncactus/pretendard@v1.3.9/dist/web/static/pretendard.min.css';
let loaded = false;

export const FONT_FAMILY =
  "Pretendard, 'Apple SD Gothic Neo', 'Malgun Gothic', 'Noto Sans KR', sans-serif";

export function loadFonts(): void {
  if (loaded || typeof document === 'undefined') return;
  loaded = true;
  const handle = delayRender('Pretendard 불러오기');
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = CSS;
  const done = () => {
    document.fonts.ready.then(() => continueRender(handle)).catch(() => continueRender(handle));
  };
  link.onload = done;
  link.onerror = () => continueRender(handle); // 오프라인이면 시스템 글꼴로
  document.head.appendChild(link);
}
