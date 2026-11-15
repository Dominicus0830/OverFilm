// Pretendard, Black Han Sans, Gaegu (모두 SIL OFL 1.1). 글꼴 파일은 저장소에 넣지 않고 렌더할 때 CDN 에서 불러온다.
// Black Han Sans·Gaegu 는 짤 장면 대사 자막(버럭·시무룩 등)에만 쓴다.
import {continueRender, delayRender} from 'remotion';

const CSS = 'https://cdn.jsdelivr.net/gh/orioncactus/pretendard@v1.3.9/dist/web/static/pretendard.min.css';
const EXTRA = 'https://fonts.googleapis.com/css2?family=Black+Han+Sans&family=Gaegu:wght@700'
  + '&family=Nanum+Myeongjo:wght@400;800&family=Nanum+Gothic:wght@400;800&family=Nanum+Pen+Script&display=block';
export const SHOUT_FONT = "'Black Han Sans', " + "Pretendard, 'Malgun Gothic', sans-serif";
export const HAND_FONT = "Gaegu, Pretendard, 'Malgun Gothic', sans-serif";
let loaded = false;

export const FONT_FAMILY =
  "Pretendard, 'Apple SD Gothic Neo', 'Malgun Gothic', 'Noto Sans KR', sans-serif";

export const SERIF_FONT = "'Nanum Myeongjo', 'Batang', serif";

/**
 * 원문 글꼴 이름 → 렌더에서 쓸 글꼴. 윈도우 글꼴(궁서·바탕·굴림…)은 렌더 PC 에 없을 수 있어
 * 비슷한 OFL 글꼴로 바꾸고, 모르는 이름은 그 이름을 먼저 시도한다.
 */
export function mapFont(name: string | undefined): string | undefined {
  if (!name) return undefined;
  const n = name.toLowerCase().replace(/\s+/g, '');
  if (/궁서|gungsuh|바탕|batang|명조|myeongjo|serif|times|georgia/.test(n) && !/sans/.test(n)) return SERIF_FONT;
  if (/나눔고딕|nanumgothic/.test(n)) return "'Nanum Gothic', " + FONT_FAMILY;
  if (/펜|손글씨|pen|brush|cursive|comic/.test(n)) return "'Nanum Pen Script', " + HAND_FONT;
  if (/impact|헤드라인|headline|blackhan/.test(n)) return SHOUT_FONT;
  if (/굴림|gulim|돋움|dotum|맑은고딕|malgun|고딕|gothic|sans|arial|tahoma|verdana|helvetica|pretendard/.test(n)) {
    return FONT_FAMILY;
  }
  return `'${name.replace(/'/g, '')}', ` + FONT_FAMILY;
}

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
