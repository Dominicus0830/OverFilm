// 프레임형 (기본 프리셋, 기획서 3.5 개정 5): 검은 배경 + 위 고정 제목 + 소제목 + 가운데 원문 미디어
// + 노란 2~4단어 자막. 원문 미디어가 비는 장면은 디시 스킨 글 카드·댓글창으로 채운다.
// 세로 위치는 화면 높이 비율: 제목 15~26%, 소제목 33~38%, 미디어 39~66%, 자막 67~72%, 78% 아래 비움.
import React from 'react';
import {
  AbsoluteFill, Audio, Img, OffthreadVideo, Sequence, interpolate, spring, staticFile, useCurrentFrame,
  useVideoConfig,
} from 'remotion';
import {BodyCard, CommentBox} from './DcSkin';
import {FONT_FAMILY, loadFonts} from './fonts';
import {RenderSpec, SpecMedia, SpecSegment, sanitize} from './spec';

loadFonts();

const src = (p: string) => (/^(https?:)?\/\//.test(p) || p.startsWith('/') ? p : staticFile(p));
const pct = (h: number, p: number) => Math.round((h * p) / 100);
export const SAFE = {title: [15, 26], sub: [33, 38], media: [39, 66], tall: [28, 66], caption: [67, 72],
  source: [73.5, 76.5], bottom: 78} as const;

function current<T extends {start: number}>(items: T[], t: number): T | null {
  let cur: T | null = null;
  for (const it of items) if (it.start <= t + 0.02) cur = it;
  return cur;
}

const Title: React.FC<{text: string; height: number}> = ({text, height}) => (
  <div style={{position: 'absolute', left: 60, right: 60, top: pct(height, SAFE.title[0]),
    height: pct(height, SAFE.title[1] - SAFE.title[0]), display: 'flex', alignItems: 'center',
    justifyContent: 'center', textAlign: 'center'}}>
    <div style={{fontSize: text.length > 16 ? 72 : 84, fontWeight: 900, color: '#fff', lineHeight: 1.2,
      letterSpacing: -2, wordBreak: 'keep-all', display: '-webkit-box', WebkitLineClamp: 2,
      WebkitBoxOrient: 'vertical', overflow: 'hidden'}}>{text}</div>
  </div>
);

const Subtitle: React.FC<{text: string; height: number; start: number}> = ({text, height, start}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const a = spring({frame: frame - Math.round(start * fps), fps, config: {damping: 200}, durationInFrames: 10});
  if (!text) return null;
  return (
    <div style={{position: 'absolute', left: 0, right: 0, top: pct(height, SAFE.sub[0]),
      height: pct(height, SAFE.sub[1] - SAFE.sub[0]), display: 'flex', alignItems: 'center',
      justifyContent: 'center', opacity: a}}>
      <span style={{fontSize: 44, fontWeight: 700, color: '#fff', background: 'rgba(255,255,255,0.14)',
        borderRadius: 999, padding: '8px 34px', letterSpacing: -1}}>{text}</span>
    </div>
  );
};

const MediaView: React.FC<{m: SpecMedia; box: {w: number; h: number}}> = ({m, box}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const t = frame / fps;
  const life = Math.max(0.5, m.end - m.start);
  const k = Math.min(1, Math.max(0, (t - m.start) / life));
  const fade = spring({frame: frame - Math.round(m.start * fps), fps, config: {damping: 200}, durationInFrames: 8});
  const aspect = m.width && m.height ? m.width / m.height : 16 / 9;
  const fit = m.display === 'tall' || m.display === 'full' ? 'contain' : 'cover';
  const common: React.CSSProperties = {position: 'absolute', inset: 0, width: '100%', height: '100%'};
  let inner: React.CSSProperties = {...common, objectFit: fit,
    transform: fit === 'cover' ? `scale(${1 + 0.06 * k})` : undefined};
  if (m.display === 'scroll') {
    const h = box.w / aspect;
    const over = Math.max(0, h - box.h);
    inner = {position: 'absolute', left: 0, top: 0, width: '100%', height: h,
      transform: `translateY(${-over * Math.min(1, Math.max(0, (t - m.start - 0.6) / Math.max(0.5, life - 1)))}px)`};
  }
  const media = (style: React.CSSProperties, muted: boolean) => (m.kind === 'video'
    ? <OffthreadVideo src={src(m.src)} muted={muted} style={style} />
    : <Img src={src(m.src)} style={style} />);
  return (
    <div style={{...common, overflow: 'hidden', opacity: fade}}>
      {fit === 'contain' ? media({...common, objectFit: 'cover', filter: 'blur(28px) brightness(0.45)',
        transform: 'scale(1.15)'}, true) : null}
      {media(inner, m.muted)}
    </div>
  );
};

const Caption: React.FC<{spec: RenderSpec; height: number}> = ({spec, height}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const t = frame / fps;
  const cap = spec.captions.find((c) => t >= c.start && t < c.end);
  if (!cap) return null;
  const pop = spring({frame: frame - Math.round(cap.start * fps), fps, config: {damping: 14, stiffness: 240},
    durationInFrames: 8});
  return (
    <div style={{position: 'absolute', left: 40, right: 40, top: pct(height, SAFE.caption[0]),
      height: pct(height, SAFE.caption[1] - SAFE.caption[0]), display: 'flex', alignItems: 'center',
      justifyContent: 'center', textAlign: 'center'}}>
      <div style={{fontSize: 74, fontWeight: 900, letterSpacing: -2, lineHeight: 1.1, wordBreak: 'keep-all',
        transform: `scale(${interpolate(pop, [0, 1], [0.85, 1])})`}}>
        {cap.words.map((w, i) => {
          const on = t >= w.start - 0.03;
          return (
            <span key={i} style={{color: w.emphasis ? '#FF7A45' : '#FFD43B', opacity: on ? 1 : 0.55,
              WebkitTextStroke: '12px #000', paintOrder: 'stroke fill', marginRight: 18}}>{w.text}</span>
          );
        })}
      </div>
    </div>
  );
};

const SegmentStage: React.FC<{spec: RenderSpec; seg: SpecSegment; next: SpecSegment | null; height: number}> = ({
  spec, seg, next, height,
}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const t = frame / fps;
  const mediaBox = {top: pct(height, SAFE.media[0]), h: pct(height, SAFE.media[1] - SAFE.media[0])};
  const tallBox = {top: pct(height, SAFE.tall[0]), h: pct(height, SAFE.tall[1] - SAFE.tall[0])};
  if (seg.kind === 'comments') {
    return (
      <div style={{position: 'absolute', left: 24, right: 24, top: tallBox.top, height: tallBox.h}}>
        <CommentBox comments={seg.comments} height={tallBox.h} blurNick={spec.nickname === 'blur'} />
      </div>
    );
  }
  if (seg.kind === 'body') {
    return (
      <div style={{position: 'absolute', left: 24, right: 24, top: tallBox.top, height: tallBox.h}}>
        <BodyCard title={spec.post_title || spec.title} subject={spec.subject} text={seg.quote}
          height={tallBox.h} start={seg.start} end={seg.end} />
      </div>
    );
  }
  // title 장면: 다음 장면의 첫 미디어를 미리 보여 주고, 없으면 본문 카드 머리만
  const items = seg.kind === 'media' ? seg.media
    : (next?.media.length ? [{...next.media[0], start: seg.start, end: seg.end, muted: true}] : []);
  const m = current(items, t);
  if (!m) {
    return (
      <div style={{position: 'absolute', left: 24, right: 24, top: mediaBox.top, height: mediaBox.h}}>
        <BodyCard title={spec.post_title || spec.title} subject={spec.subject} text="" height={mediaBox.h}
          start={seg.start} end={seg.end} />
      </div>
    );
  }
  const full = m.display === 'full';
  const box = full ? {top: 0, h: height} : (m.display === 'tall' || m.display === 'scroll') ? tallBox : mediaBox;
  return (
    <div style={{position: 'absolute', left: 0, right: 0, top: box.top, height: box.h}}>
      <MediaView key={m.src + m.start} m={m} box={{w: 1080, h: box.h}} />
    </div>
  );
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
  const m = seg?.kind === 'media' ? current(seg.media, t) : null;
  const fullScreen = m?.display === 'full';
  const hideSub = !seg || seg.kind === 'comments' || seg.kind === 'body' || fullScreen
    || m?.display === 'tall' || m?.display === 'scroll';
  const speaking = (f: number) => spec.scenes.some((s) => f / fps >= s.start && f / fps <= s.end);
  return (
    <AbsoluteFill style={{fontFamily: FONT_FAMILY, background: '#000'}}>
      {seg ? <SegmentStage spec={spec} seg={seg} next={next} height={height} /> : null}
      {fullScreen ? null : <Title text={spec.title} height={height} />}
      {!hideSub && seg ? <Subtitle text={seg.subtitle} height={height} start={seg.start} /> : null}
      <Caption spec={spec} height={height} />
      <div style={{position: 'absolute', left: 0, right: 0, top: pct(height, SAFE.source[0]), textAlign: 'center',
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
