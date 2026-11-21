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
/** 글쓴이가 꾸민 글자 조각 (원문 그대로: 색·배경·크기 px·글꼴 이름·굵게·기울임·밑줄·취소선). */
export type StyledRun = {t: string; color?: string; bg?: string; size?: number; font?: string; bold?: boolean;
  italic?: boolean; underline?: boolean; strike?: boolean};
export type StyledLine = StyledRun[];
export type SpecMedia = {
  src: string; kind: 'image' | 'video';
  display: 'fill' | 'crop' | 'full' | 'tall' | 'scroll';
  start: number; end: number; width: number; height: number; muted: boolean;
  playback_rate?: number; // 음성·칸 길이에 맞춘 클립 배속 (0.75~1.25)
  number: number; caption: string; // 원문 사진 번호, 사진 바로 아래 본문 줄 (스크린샷처럼)
  caption_runs: StyledLine[] | null; // 원문에 꾸민 줄이면 줄마다 꾸밈
};
export type SpecPost = {title: string; subject: string; nick: string; ip: string; icon: string; date: string};
export type SpecComment = {
  no: number; nick: string; ip: string; icon: '' | 'fix' | 'half'; text: string; depth: number;
  dccon: {src: string | null; name: string} | null; appear: number; read: [number, number] | null;
  page: number; // 스크린샷 한 장 번호 (원댓글과 답글은 같은 장)
};
export type SpecSegment = {
  kind: 'title' | 'media' | 'body' | 'comments' | 'reaction';
  start: number; end: number; subtitle: string; tts: boolean;
  media: SpecMedia[]; comments: SpecComment[]; quote: string; quote_runs: StyledLine[] | null;
  image: string | null; say: string; // reaction: 짤·영웅 프로필 + 대사 (자막은 captions 에)
};
export type SayStyle = 'calm' | 'shout' | 'question' | 'sad' | 'smug' | 'band'; // band = 흰 띠 한 줄
export type SpecCaption = {start: number; end: number; seg: number; words: SpecWord[]; style: SayStyle | null;
  runs: StyledLine | null}; // 원문 줄을 그대로 쓴 짤 장면 대사면 원문 꾸밈
/** 화면 배치 (편집 유형 프리셋): 세로 위치는 화면 높이 %. 없으면 옛 프레임형(제목 바 + 노란 자막). */
export type SpecLayout = {
  title_bar: boolean; caption: 'yellow' | 'band' | 'none';
  stage: [number, number]; title: [number, number]; caption_box: [number, number];
  source_top: number; safe_bottom: number;
};
export const LEGACY_LAYOUT: SpecLayout = {title_bar: true, caption: 'yellow', stage: [28, 66], title: [15, 26],
  caption_box: [67, 72], source_top: 75.5, safe_bottom: 78};
export type RenderSpec = {
  layout: SpecLayout;
  edit_type: string;
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
  post: SpecPost;
  subject: string;
  source: string;
  source_text?: string; // 출처를 꺼도 원래 문구를 기억 (다시 켤 때)
  audio: string | null; // public 폴더 기준 경로
  background: string | null;
  /** 배경 영상이 SDR 이라고 Python 이 ffprobe 로 확인함: 색 변환(tone mapping)을 건너뛴다 */
  background_sdr: boolean;
  theme: {dark: boolean; accent: string};
  scenes: SpecScene[];
  /** 'stage' = 장면만 (제목·자막·출처·소리 없이, 투명 바탕): OverFilm 편집기로 넘길 그림을 그릴 때 */
  layers?: 'all' | 'stage';
};

export const EMPTY_SPEC: RenderSpec = {
  layout: LEGACY_LAYOUT,
  edit_type: '',
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
  post: {title: '', subject: '', nick: 'ㅇㅇ', ip: '', icon: '', date: ''},
  subject: '일반',
  source: '출처: DC인사이드 예시 갤러리',
  audio: null,
  background: null,
  background_sdr: false,
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

const COLOR_RE = /^(#[0-9a-fA-F]{3,8}|[a-zA-Z]{3,20})$/;

/** 꾸밈 조각도 믿지 않는다: 색은 #hex·이름만, 크기는 6~200px, 글꼴 이름은 글자만. */
export function sanitizeRuns(v: unknown, maxRuns = 40): StyledLine | null {
  if (!Array.isArray(v)) return null;
  const out: StyledLine = [];
  for (const r of v.slice(0, maxRuns)) {
    if (!r || typeof r.t !== 'string') continue;
    const run: StyledRun = {t: r.t.slice(0, 300)};
    if (typeof r.color === 'string' && COLOR_RE.test(r.color)) run.color = r.color;
    if (typeof r.bg === 'string' && COLOR_RE.test(r.bg)) run.bg = r.bg;
    if (typeof r.size === 'number' && Number.isFinite(r.size)) run.size = Math.min(200, Math.max(6, r.size));
    if (typeof r.font === 'string' && /^[\p{L}\p{N} _-]{1,40}$/u.test(r.font)) run.font = r.font;
    for (const k of ['bold', 'italic', 'underline', 'strike'] as const) if (r[k] === true) run[k] = true;
    out.push(run);
  }
  return out.length ? out : null;
}

const sanitizeLines = (v: unknown): StyledLine[] | null => {
  if (!Array.isArray(v)) return null;
  const lines = v.slice(0, 60).map((l) => sanitizeRuns(l) ?? []);
  return lines.some((l) => l.length) ? lines : null;
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
    kind: (['title', 'media', 'body', 'comments', 'reaction'] as const).includes(g?.kind) ? g.kind : 'body',
    image: typeof g?.image === 'string' && g.image ? g.image : null, say: str(g?.say).slice(0, 60),
    start: t(g?.start), end: t(g?.end), subtitle: str(g?.subtitle).slice(0, 40), tts: g?.tts === true,
    quote: str(g?.quote).slice(0, 800), quote_runs: sanitizeLines(g?.quote_runs),
    media: (Array.isArray(g?.media) ? g.media.slice(0, 6) : []).map((m) => ({
      src: str(m?.src), kind: m?.kind === 'video' ? 'video' : 'image',
      display: (['fill', 'crop', 'full', 'tall', 'scroll'] as const).includes(m?.display) ? m.display : 'fill',
      start: t(m?.start), end: t(m?.end), width: num(m?.width, 0, 0, 20000),
      height: num(m?.height, 0, 0, 20000), muted: m?.muted !== false,
      number: Math.round(num(m?.number, 0, 0, 999)), caption: str(m?.caption),
      playback_rate: num(m?.playback_rate, 1, 0.5, 2),
      caption_runs: sanitizeLines(m?.caption_runs),
    })),
    comments: (Array.isArray(g?.comments) ? g.comments.slice(0, 60) : []).map((c) => ({
      no: num(c?.no, 0, 0, 10000), nick: str(c?.nick).slice(0, 30), ip: str(c?.ip).slice(0, 20),
      icon: c?.icon === 'fix' || c?.icon === 'half' ? c.icon : '', text: str(c?.text),
      depth: num(c?.depth, 0, 0, 3),
      dccon: c?.dccon ? {src: typeof c.dccon.src === 'string' ? c.dccon.src : null, name: str(c.dccon.name)} : null,
      appear: t(c?.appear),
      read: Array.isArray(c?.read) && c.read.length === 2 ? [t(c.read[0]), t(c.read[1])] : null,
      page: Math.round(num(c?.page, 0, 0, 50)),
    })),
  }));
  const captions: SpecCaption[] = (Array.isArray(s.captions) ? s.captions.slice(0, 400) : []).map((c) => ({
    start: t(c?.start), end: t(c?.end), seg: num(c?.seg, 0, 0, 20), words: words(c?.words).slice(0, 8),
    style: (['calm', 'shout', 'question', 'sad', 'smug', 'band'] as readonly unknown[]).includes(c?.style) ? c.style as SayStyle : null,
    runs: sanitizeRuns(c?.runs),
  }));
  const pair = (v: unknown, d: [number, number]): [number, number] => {
    const a = Array.isArray(v) && v.length === 2 ? v : d;
    const lo = num(a[0], d[0], 0, 100);
    const hi = num(a[1], d[1], 0, 100);
    return [Math.min(lo, hi), Math.max(lo, hi)];
  };
  const ly = s.layout;
  const layout: SpecLayout = ly ? {
    title_bar: ly.title_bar === true,
    caption: ly.caption === 'band' || ly.caption === 'none' ? ly.caption : 'yellow',
    stage: pair(ly.stage, LEGACY_LAYOUT.stage), title: pair(ly.title, LEGACY_LAYOUT.title),
    caption_box: pair(ly.caption_box, LEGACY_LAYOUT.caption_box),
    source_top: num(ly.source_top, 75.5, 0, 100), safe_bottom: num(ly.safe_bottom, 78, 50, 100),
  } : LEGACY_LAYOUT;
  return {
    layout,
    edit_type: str(s.edit_type).slice(0, 32),
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
    post: {title: str(s.post?.title), subject: str(s.post?.subject), nick: str(s.post?.nick, 'ㅇㅇ').slice(0, 30),
      ip: str(s.post?.ip).slice(0, 20), icon: str(s.post?.icon).slice(0, 8), date: str(s.post?.date).slice(0, 30)},
    subject: str(s.subject),
    source: str(s.source), source_text: str(s.source_text),
    audio: typeof s.audio === 'string' ? s.audio : null,
    background: typeof s.background === 'string' ? s.background : null,
    background_sdr: s.background_sdr === true,
    theme: {dark: s.theme?.dark !== false, accent: str(s.theme?.accent, '#FFD43B') || '#FFD43B'},
    layers: s.layers === 'stage' ? 'stage' : 'all',
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
