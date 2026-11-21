// 프레임형 (기본 프리셋, 기획서 3.5 개정 5): 검은 배경 + 위 고정 제목 + 가운데 원문 + 노란 2~4단어 자막.
// 사진·글·댓글은 디시 화면 스크린샷처럼 그린다 (2026-11-15 피드백: 확대 효과·글자별 색 없음,
// 소제목 자리까지 써서 스크린샷을 28~66% 에 크게).
// 세로 위치는 화면 높이 비율: 제목 15~26%, 스크린샷 28~66%, 자막 67~72%, 출처 75.5%, 78% 아래 비움.
import React from 'react';
import {
  AbsoluteFill, Audio, Img, Sequence, staticFile, useCurrentFrame, useVideoConfig,
} from 'remotion';
import {
  BodyText, CommentPage, PhotoBlock, PostHead, SKIN_W, Scaled, photoHeight, runCss, textHeight, useScroll,
} from './DcSkin';
import {FONT_FAMILY, HAND_FONT, SHOUT_FONT, loadFonts} from './fonts';
import {RenderSpec, SayStyle, SpecMedia, SpecSegment, StyledLine, sanitize} from './spec';

loadFonts();

const src = (p: string) => (/^(https?:)?\/\//.test(p) || p.startsWith('/') ? p : staticFile(p));
const pct = (h: number, p: number) => Math.round((h * p) / 100);
export const SAFE = {title: [15, 26], tall: [28, 66], caption: [67, 72],
  source: [75.5, 77.5], bottom: 78} as const;
type Safe = {title: readonly number[]; tall: readonly number[]; caption: readonly number[];
  say: readonly number[]; source: readonly number[]; bottom: number};

/** 화면 배치(편집 유형)에서 세로 안전영역 %를 얻는다. 제목 바가 있으면 옛 위치(스크린샷 아래 자막), 없으면
 * 스크린샷을 크게 쓰고 대사·띠 자막은 그 아래쪽에 겹쳐 둔다 (참고 쇼츠처럼 짤 안에 붙은 띠). */
function safeOf(spec: RenderSpec): Safe {
  const l = spec.layout;
  const say = l.title_bar ? [l.stage[1] + 0.5, l.source_top - 0.5] : l.caption_box;
  return {title: l.title, tall: l.stage, caption: l.caption_box, say, source: [l.source_top, l.source_top + 2],
    bottom: l.safe_bottom};
}

function current<T extends {start: number}>(items: T[], t: number): T | null {
  let cur: T | null = null;
  for (const it of items) if (it.start <= t + 0.02) cur = it;
  return cur;
}

const Title: React.FC<{text: string; height: number; safe: Safe}> = ({text, height, safe}) => (
  <div style={{position: 'absolute', left: 60, right: 60, top: pct(height, safe.title[0]),
    height: pct(height, safe.title[1] - safe.title[0]), display: 'flex', alignItems: 'center',
    justifyContent: 'center', textAlign: 'center'}}>
    <div style={{fontSize: text.length > 16 ? 72 : 84, fontWeight: 900, color: '#fff', lineHeight: 1.2,
      letterSpacing: -2, wordBreak: 'keep-all', display: '-webkit-box', WebkitLineClamp: 2,
      WebkitBoxOrient: 'vertical', overflow: 'hidden'}}>{text}</div>
  </div>
);

// 짤 장면 대사: 상황에 맞춰 색·크기·글꼴 (2026-11-15 사용자 의견). 흔들기·확대 같은 연출은 없음.
const SAY_LOOK: Record<Exclude<SayStyle, 'band'>, React.CSSProperties> = {
  calm: {fontFamily: FONT_FAMILY, fontWeight: 800, fontSize: 80, color: '#FFFFFF', WebkitTextStroke: '12px #000'},
  question: {fontFamily: SHOUT_FONT, fontWeight: 400, fontSize: 100, color: '#FFFFFF', WebkitTextStroke: '14px #000'},
  shout: {fontFamily: SHOUT_FONT, fontWeight: 400, fontSize: 124, color: '#FF3B30', WebkitTextStroke: '16px #000',
    letterSpacing: 2},
  sad: {fontFamily: HAND_FONT, fontWeight: 700, fontSize: 100, color: '#9CC9FF', WebkitTextStroke: '12px #000'},
  smug: {fontFamily: SHOUT_FONT, fontWeight: 400, fontSize: 108, color: '#FFD43B', WebkitTextStroke: '14px #000'},
};

const SayCaption: React.FC<{text: string; style: Exclude<SayStyle, 'band'>; height: number; safe: Safe}> = ({
  text, style, height, safe,
}) => (
  <div style={{position: 'absolute', left: 30, right: 30, top: pct(height, safe.say[0]),
    height: pct(height, safe.say[1] - safe.say[0]), display: 'flex', alignItems: 'center',
    justifyContent: 'center', textAlign: 'center'}}>
    <div style={{lineHeight: 1.1, wordBreak: 'keep-all', paintOrder: 'stroke fill', ...SAY_LOOK[style],
      // 긴 줄은 두 줄 안에 들어가게 줄인다
      fontSize: Math.max(58, Math.round(Number(SAY_LOOK[style].fontSize)
        * Math.min(1, 12 / Math.max(1, text.replace(/\s/g, '').length)) ** 0.75))}}>
      {text}</div>
  </div>
);

/** 흰 띠 한 줄 자막: 참고 쇼츠의 짤·클립 아래에 붙은 흰 박스 (한 줄, 가운데). 글자 수에 맞춰 줄인다. */
const BandCaption: React.FC<{text: string; height: number; safe: Safe}> = ({text, height, safe}) => (
  <div style={{position: 'absolute', left: 0, right: 0, top: pct(height, safe.caption[0]),
    height: pct(height, safe.caption[1] - safe.caption[0]), display: 'flex', alignItems: 'center',
    justifyContent: 'center'}}>
    <div style={{background: '#fff', color: '#111', fontWeight: 800, padding: '10px 28px', maxWidth: '94%',
      textAlign: 'center', whiteSpace: 'nowrap', lineHeight: 1.25, letterSpacing: -1,
      fontSize: Math.max(34, Math.min(60, Math.round(1000 / Math.max(8, text.length))))}}>{text}</div>
  </div>
);

// 원문 줄을 그대로 쓴 대사: 글쓴이가 꾸민 색·크기·글꼴·굵게·기울임을 따른다 (2026-11-15 사용자 의견).
// 바탕은 차분한 흰 글씨(calm)이고 원문 꾸밈이 덮어쓴다. 사진 위에서도 읽히게 검은 테두리는 남긴다.
const RichSayCaption: React.FC<{runs: StyledLine; height: number; safe: Safe}> = ({runs, height, safe}) => {
  const base = SAY_LOOK.calm;
  const chars = runs.reduce((n, r) => n + r.t.replace(/\s/g, '').length, 0);
  const fit = Math.min(1, 12 / Math.max(1, chars)) ** 0.75;
  // 원문 크기 비율: 디시 기본 13~16px 이 80, 36px 이면 1.5배까지
  const sizeOf = (px?: number) => Math.max(58, Math.round(Number(base.fontSize) * fit
    * Math.min(1.5, Math.max(1, (px ?? 16) / 20))));
  return (
    <div style={{position: 'absolute', left: 30, right: 30, top: pct(height, safe.say[0]),
      height: pct(height, safe.say[1] - safe.say[0]), display: 'flex', alignItems: 'center',
      justifyContent: 'center', textAlign: 'center'}}>
      <div style={{lineHeight: 1.1, wordBreak: 'keep-all', paintOrder: 'stroke fill', ...base, fontSize: sizeOf()}}>
        {runs.map((r, k) => {
          const css = runCss(r);
          return (
            <span key={k} style={{...css, fontWeight: r.bold ? 900 : base.fontWeight,
              fontFamily: css.fontFamily ?? base.fontFamily, color: css.color ?? base.color,
              background: undefined, fontSize: sizeOf(r.size)}}>{r.t}</span>
          );
        })}
      </div>
    </div>
  );
};

const Caption: React.FC<{spec: RenderSpec; height: number; safe: Safe}> = ({spec, height, safe}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const t = frame / fps;
  const cap = spec.captions.find((c) => t >= c.start && t < c.end);
  if (!cap) return null;
  if (cap.runs) return <RichSayCaption runs={cap.runs} height={height} safe={safe} />;
  if (cap.style === 'band') return <BandCaption text={cap.words.map((w) => w.text).join(' ')} height={height} safe={safe} />;
  if (cap.style) return <SayCaption text={cap.words.map((w) => w.text).join(' ')} style={cap.style} height={height} safe={safe} />;
  // 한 색으로 2~4단어를 통째로 보여 준다 (글자별 색 바뀜·확대 없음)
  return (
    <div style={{position: 'absolute', left: 40, right: 40, top: pct(height, safe.caption[0]),
      height: pct(height, safe.caption[1] - safe.caption[0]), display: 'flex', alignItems: 'center',
      justifyContent: 'center', textAlign: 'center'}}>
      <div style={{fontSize: 74, fontWeight: 900, letterSpacing: -2, lineHeight: 1.1, wordBreak: 'keep-all',
        color: '#FFD43B', WebkitTextStroke: '12px #000', paintOrder: 'stroke fill'}}>
        {cap.words.map((w) => w.text).join(' ')}
      </div>
    </div>
  );
};

/** 사진 장면: 디시 글 스크린샷처럼 (첫 사진이면 글 머리도 같이). 화면보다 길면 천천히 내려감. */
const PhotoPage: React.FC<{spec: RenderSpec; m: SpecMedia; head: boolean; view: number; start: number;
  end: number}> = ({spec, m, head, view, start, end}) => {
  const k = 1080 / SKIN_W;
  const contentH = (head ? 52 : 0) + photoHeight(m);
  const y = useScroll(contentH, view / k, start, end);
  return (
    <Scaled width={1080} height={view} scrollY={y}>
      {head ? <PostHead post={spec.post} /> : null}
      <PhotoBlock m={m} />
    </Scaled>
  );
};

const TextPage: React.FC<{spec: RenderSpec; text: string; runs?: StyledLine[] | null; head: boolean; view: number;
  start: number; end: number}> = ({spec, text, runs, head, view, start, end}) => {
  const k = 1080 / SKIN_W;
  const y = useScroll((head ? 52 : 0) + textHeight(text, runs), view / k, start, end);
  // 짧은 글: 흰 바탕을 글 크기에 맞춰 화면 정중앙에, 글자는 키운다 (2026-11-21 사용자 의견).
  // 키우는 배율 z: 가상 화면 폭을 1/z 로 줄이고 CSS zoom 으로 키워, 실제 폭은 1080 그대로 두고 줄바꿈만 다시 한다.
  const z = head ? 1 : shortTextZoom(text, runs, view / k);
  if (z > 1) {
    return (
      <div style={{position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center'}}>
        <div style={{width: SKIN_W / z, zoom: k * z, background: '#fff', color: '#333', fontSize: 13,
          lineHeight: 1.5}}>
          <BodyText text={text} runs={runs} />
        </div>
      </div>
    );
  }
  return (
    <Scaled width={1080} height={view} scrollY={y}>
      {head ? <PostHead post={spec.post} /> : null}
      <BodyText text={text} runs={runs} />
    </Scaled>
  );
};

/** 글이 무대의 85% 안에 들어가는 가장 큰 배율 (1~3). 1 이면 지금처럼 꽉 찬 디시 화면. */
function shortTextZoom(text: string, runs: StyledLine[] | null | undefined, viewVirtual: number): number {
  const base = textHeight(text, runs);
  if (base > viewVirtual * 0.5) return 1; // 이미 화면 절반 넘게 차면 그대로
  const paras = text.split('\n');
  const big = runs ? Math.max(1, ...runs.flat().map((r) => (r.size && r.size > 16 ? Math.min(30, r.size * 0.65) / 13 : 1)))
    : 1;
  for (let z = 3; z > 1; z = Math.round((z - 0.1) * 10) / 10) {
    // 가상 폭이 1/z 로 줄면 한 줄 글자 수도 1/z
    const rows = paras.reduce((n, p) => n + Math.max(1, Math.ceil((p.length * z * big) / 36)), 0);
    const h = (20 + rows * 13 * 1.5 * big) * z; // 키운 뒤 가상 높이
    if (h <= viewVirtual * 0.85) return z;
  }
  return 1;
}

const SegmentStage: React.FC<{spec: RenderSpec; seg: SpecSegment; next: SpecSegment | null; height: number;
  safe: Safe}> = ({spec, seg, next, height, safe}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const t = frame / fps;
  const box = {top: pct(height, safe.tall[0]), h: pct(height, safe.tall[1] - safe.tall[0])};
  const wrap = (child: React.ReactNode) => (
    <div style={{position: 'absolute', left: 0, right: 0, top: box.top, height: box.h, overflow: 'hidden'}}>
      {child}
    </div>
  );
  if (seg.kind === 'comments') {
    const pages = [...new Set(seg.comments.map((c) => c.page))];
    let page = pages[0] ?? 0;
    for (const p of pages) {
      const first = seg.comments.find((c) => c.page === p);
      if (first && first.appear <= t + 0.02) page = p;
    }
    return wrap(
      <Scaled width={1080} height={box.h}>
        <CommentPage comments={seg.comments.filter((c) => c.page === page)} blurNick={spec.nickname === 'blur'} />
      </Scaled>,
    );
  }
  if (seg.kind === 'reaction') {
    // 짤·영웅 프로필: 검은 바탕 가운데에 통째로 (확대·흔들기 없음). 대사는 아래 자막으로.
    return wrap(seg.image ? (
      <div style={{position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center'}}>
        <Img src={src(seg.image)} style={{width: '100%', height: '100%', objectFit: 'contain'}} />
      </div>
    ) : null);
  }
  if (seg.kind === 'body') {
    return wrap(<TextPage spec={spec} text={seg.quote} runs={seg.quote_runs} head={false} view={box.h} start={seg.start} end={seg.end} />);
  }
  // 제목 장면: 글 머리 + 다음 장면 첫 사진 (실제 글 첫 화면처럼)
  const items = seg.kind === 'media' ? seg.media
    : (next?.media.length ? [{...next.media[0], start: seg.start, end: seg.end, muted: true}] : []);
  const m = current(items, t);
  if (!m) {
    // 사진 없는 글: 디시 글 머리만 스크린샷처럼 (아래를 흰 칸으로 채우지 않음)
    const k = 1080 / SKIN_W;
    const hh = Math.round(64 * k);
    return wrap(
      <div style={{position: 'absolute', left: 0, right: 0, top: Math.round((box.h - hh) / 2), height: hh}}>
        <Scaled width={1080} height={hh}><PostHead post={spec.post} /></Scaled>
      </div>,
    );
  }
  const head = seg.kind === 'title' || m.number === 1;
  return wrap(<PhotoPage key={m.src + m.start} spec={spec} m={m} head={head} view={box.h}
    start={m.start} end={m.end} />);
};

export const FrameStory: React.FC<Record<string, unknown>> = (props) => {
  const spec = sanitize(props);
  const frame = useCurrentFrame();
  const {fps, height} = useVideoConfig();
  const t = frame / fps;
  const idx = spec.segments.findIndex((s, i) => s.start <= t + 0.02
    && (i === spec.segments.length - 1 || spec.segments[i + 1].start > t + 0.02));
  const seg = idx >= 0 ? spec.segments[idx] : null;
  const next = idx >= 0 && idx + 1 < spec.segments.length ? spec.segments[idx + 1] : null;
  const safe = safeOf(spec);
  const speaking = (f: number) => spec.scenes.some((s) => f / fps >= s.start && f / fps <= s.end);
  if (spec.layers === 'stage') {
    // OverFilm 편집기로 넘길 장면 그림: 제목·자막·출처·소리는 편집기에서 글자·소리 클립으로 따로 놓는다.
    // 짤 장면은 원본 그림을 편집기에 그대로 놓으므로 그리지 않는다.
    return (
      <AbsoluteFill style={{fontFamily: FONT_FAMILY, background: 'transparent'}}>
        {seg && seg.kind !== 'reaction' ? <SegmentStage spec={spec} seg={seg} next={next} height={height} safe={safe} /> : null}
      </AbsoluteFill>
    );
  }
  return (
    <AbsoluteFill style={{fontFamily: FONT_FAMILY, background: '#000'}}>
      {seg ? <SegmentStage spec={spec} seg={seg} next={next} height={height} safe={safe} /> : null}
      {spec.layout.title_bar ? <Title text={spec.title} height={height} safe={safe} /> : null}
      <Caption spec={spec} height={height} safe={safe} />
      <div style={{position: 'absolute', left: 0, right: 0, top: pct(height, safe.source[0]), textAlign: 'center',
        fontSize: 28, color: 'rgba(255,255,255,0.6)'}}>{spec.source}</div>
      {spec.audio ? <Audio src={src(spec.audio)} /> : null}
      {spec.bgm ? (
        <Sequence from={0}>
          {/* 말할 때는 −12dB 로 줄인다 (덕킹) */}
          <Audio src={src(spec.bgm)} loop volume={(f) => (speaking(f) ? 0.07 : 0.28)} />
        </Sequence>
      ) : null}
    </AbsoluteFill>
  );
};
