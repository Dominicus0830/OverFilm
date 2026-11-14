// render_spec.json 형식. Python(core/overshorts/render/spec.py)이 만들고 여기서는 화면만 그린다.
export type SpecWord = {text: string; start: number; end: number; emphasis?: boolean};
export type SpecScene = {
  line?: number; // 대본 줄 번호 (검수 화면의 '그 문장만 듣기')
  part: 'hook' | 'body' | 'comment' | 'outro';
  speaker: 'narrator' | 'comment';
  text: string;
  start: number;
  end: number;
  words: SpecWord[];
};
export type SpecMedia = {
  src: string; kind: 'image' | 'video';
  display: 'fill' | 'crop' | 'full' | 'tall' | 'scroll';
  start: number; end: number; width: number; height: number; muted: boolean;
};
export type SpecComment = {
  no: number; nick: string; ip: string; icon: '' | 'fix' | 'half'; text: string; depth: number;
  dccon: {src: string | null; name: string} | null; appear: number; read: [number, number] | null;
};
export type SpecSegment = {
  kind: 'title' | 'media' | 'body' | 'comments';
  start: number; end: number; subtitle: string; tts: boolean;
  media: SpecMedia[]; comments: SpecComment[]; quote: string;
};
export type SpecCaption = {start: number; end: number; seg: number; words: SpecWord[]};
export type RenderSpec = {
  version: 1 | 2;
  preset: 'frame' | 'card';
  segments: SpecSegment[];
  captions: SpecCaption[];
  bgm: string | null;
  nickname: 'original' | 'blur' | 'replace';
  fps: number;
  width: number;
  height: number;
  duration: number; // 초
  title: string;
  post_title: string; // 원문 제목 (디시 스킨 글 카드)
  subject: string;
  source: string;
  audio: string | null; // public 폴더 기준 경로
  background: string | null;
  theme: {dark: boolean; accent: string};
  scenes: SpecScene[];
};

export const EMPTY_SPEC: RenderSpec = {
  version: 1,
  preset: 'card',
  segments: [],
  captions: [],
  bgm: null,
  nickname: 'original',
  fps: 60,
  width: 1080,
  height: 1920,
  duration: 6,
  title: '받은 힐량 표시',
  post_title: '',
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
  const t = (v: unknown) => num(v, 0, 0, 180);
  const words = (ws: unknown): SpecWord[] => (Array.isArray(ws) ? ws.slice(0, 300) : []).map((w) => ({
    text: str(w?.text), start: t(w?.start), end: t(w?.end), emphasis: w?.emphasis === true,
  }));
  const segments: SpecSegment[] = (Array.isArray(s.segments) ? s.segments.slice(0, 20) : []).map((g) => ({
    kind: (['title', 'media', 'body', 'comments'] as const).includes(g?.kind) ? g.kind : 'body',
    start: t(g?.start), end: t(g?.end), subtitle: str(g?.subtitle).slice(0, 40), tts: g?.tts === true,
    quote: str(g?.quote).slice(0, 800),
    media: (Array.isArray(g?.media) ? g.media.slice(0, 6) : []).map((m) => ({
      src: str(m?.src), kind: m?.kind === 'video' ? 'video' : 'image',
      display: (['fill', 'crop', 'full', 'tall', 'scroll'] as const).includes(m?.display) ? m.display : 'fill',
      start: t(m?.start), end: t(m?.end), width: num(m?.width, 0, 0, 20000),
      height: num(m?.height, 0, 0, 20000), muted: m?.muted !== false,
    })),
    comments: (Array.isArray(g?.comments) ? g.comments.slice(0, 12) : []).map((c) => ({
      no: num(c?.no, 0, 0, 10000), nick: str(c?.nick).slice(0, 30), ip: str(c?.ip).slice(0, 20),
      icon: c?.icon === 'fix' || c?.icon === 'half' ? c.icon : '', text: str(c?.text),
      depth: num(c?.depth, 0, 0, 3),
      dccon: c?.dccon ? {src: typeof c.dccon.src === 'string' ? c.dccon.src : null, name: str(c.dccon.name)} : null,
      appear: t(c?.appear),
      read: Array.isArray(c?.read) && c.read.length === 2 ? [t(c.read[0]), t(c.read[1])] : null,
    })),
  }));
  const captions: SpecCaption[] = (Array.isArray(s.captions) ? s.captions.slice(0, 400) : []).map((c) => ({
    start: t(c?.start), end: t(c?.end), seg: num(c?.seg, 0, 0, 20), words: words(c?.words).slice(0, 8),
  }));
  return {
    version: s.version === 2 ? 2 : 1,
    preset: s.preset === 'frame' ? 'frame' : 'card',
    segments,
    captions,
    bgm: typeof s.bgm === 'string' ? s.bgm : null,
    nickname: s.nickname === 'blur' || s.nickname === 'replace' ? s.nickname : 'original',
    fps: Math.round(num(s.fps, 60, 24, 60)),
    width: Math.round(num(s.width, 1080, 360, 2160)),
    height: Math.round(num(s.height, 1920, 640, 3840)),
    duration: num(s.duration, 6, 1, 180),
    title: str(s.title),
    post_title: str(s.post_title),
    subject: str(s.subject),
    source: str(s.source),
    audio: typeof s.audio === 'string' ? s.audio : null,
    background: typeof s.background === 'string' ? s.background : null,
    theme: {dark: s.theme?.dark !== false, accent: str(s.theme?.accent, '#FFD43B') || '#FFD43B'},
    scenes: scenes.map((sc) => ({
      line: Math.round(num(sc?.line, 0, 0, 1000)),
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
