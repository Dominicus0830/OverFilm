// render_spec.json 형식. Python(core/overshorts/render/spec.py)이 만들고 여기서는 화면만 그린다.
export type SpecWord = {text: string; start: number; end: number; emphasis?: boolean};
export type SpecScene = {
  part: 'hook' | 'body' | 'comment' | 'outro';
  speaker: 'narrator' | 'comment';
  text: string;
  start: number;
  end: number;
  words: SpecWord[];
};
export type RenderSpec = {
  version: 1;
  fps: number;
  width: number;
  height: number;
  duration: number; // 초
  title: string;
  subject: string;
  source: string;
  audio: string | null; // public 폴더 기준 경로
  background: string | null;
  theme: {dark: boolean; accent: string};
  scenes: SpecScene[];
};

export const EMPTY_SPEC: RenderSpec = {
  version: 1,
  fps: 60,
  width: 1080,
  height: 1920,
  duration: 6,
  title: '받은 힐량 표시',
  subject: '일반',
  source: '출처: DC인사이드 예시 갤러리',
  audio: null,
  background: null,
  theme: {dark: true, accent: '#FFD43B'},
  scenes: [
    {part: 'hook', speaker: 'narrator', text: '힐러가 받은 힐량이 3천이었다.', start: 0, end: 2.4,
      words: [
        {text: '힐러가', start: 0.1, end: 0.6}, {text: '받은', start: 0.6, end: 1.0},
        {text: '힐량이', start: 1.0, end: 1.5}, {text: '3천이었다.', start: 1.5, end: 2.3, emphasis: true},
      ]},
    {part: 'comment', speaker: 'comment', text: '이거 진짜 필요했음', start: 2.6, end: 4.2,
      words: [
        {text: '이거', start: 2.7, end: 3.0}, {text: '진짜', start: 3.0, end: 3.4},
        {text: '필요했음', start: 3.4, end: 4.1},
      ]},
  ],
};

/** 들어온 값을 믿지 않는다: 범위를 자르고 빠진 값은 채운다. */
export function sanitize(input: unknown): RenderSpec {
  const s = (input ?? {}) as Partial<RenderSpec>;
  const num = (v: unknown, d: number, lo: number, hi: number) =>
    typeof v === 'number' && Number.isFinite(v) ? Math.min(hi, Math.max(lo, v)) : d;
  const str = (v: unknown, d = '') => (typeof v === 'string' ? v.slice(0, 500) : d);
  const scenes = Array.isArray(s.scenes) ? s.scenes.slice(0, 200) : [];
  return {
    version: 1,
    fps: Math.round(num(s.fps, 60, 24, 60)),
    width: Math.round(num(s.width, 1080, 360, 2160)),
    height: Math.round(num(s.height, 1920, 640, 3840)),
    duration: num(s.duration, 6, 1, 180),
    title: str(s.title),
    subject: str(s.subject),
    source: str(s.source),
    audio: typeof s.audio === 'string' ? s.audio : null,
    background: typeof s.background === 'string' ? s.background : null,
    theme: {dark: s.theme?.dark !== false, accent: str(s.theme?.accent, '#FFD43B') || '#FFD43B'},
    scenes: scenes.map((sc) => ({
      part: (['hook', 'body', 'comment', 'outro'] as const).includes(sc?.part) ? sc.part : 'body',
      speaker: sc?.speaker === 'comment' ? 'comment' : 'narrator',
      text: str(sc?.text),
      start: num(sc?.start, 0, 0, 180),
      end: num(sc?.end, 0, 0, 180),
      words: (Array.isArray(sc?.words) ? sc.words.slice(0, 300) : []).map((w) => ({
        text: str(w?.text),
        start: num(w?.start, 0, 0, 180),
        end: num(w?.end, 0, 0, 180),
        emphasis: w?.emphasis === true,
      })),
    })),
  };
}
