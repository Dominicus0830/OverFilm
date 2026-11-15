// 프레임형 (기본 프리셋, 기획서 3.5 개정 5): 검은 배경 + 위 고정 제목 + 가운데 원문 + 노란 2~4단어 자막.
// 사진·글·댓글은 디시 화면 스크린샷처럼 그린다 (2026-11-15 피드백: 확대 효과·글자별 색 없음,
// 소제목 자리까지 써서 스크린샷을 28~66% 에 크게).
// 세로 위치는 화면 높이 비율: 제목 15~26%, 스크린샷 28~66%, 자막 67~72%, 출처 74%, 78% 아래 비움.
import React from 'react';
import {
  AbsoluteFill, Audio, Img, Sequence, staticFile, useCurrentFrame, useVideoConfig,
} from 'remotion';
import {
  BodyText, CommentPage, PhotoBlock, PostHead, SKIN_W, Scaled, photoHeight, textHeight, useScroll,
} from './DcSkin';
import {FONT_FAMILY, loadFonts} from './fonts';
import {RenderSpec, SpecMedia, SpecSegment, sanitize} from './spec';

loadFonts();

const src = (p: string) => (/^(https?:)?\/\//.test(p) || p.startsWith('/') ? p : staticFile(p));
const pct = (h: number, p: number) => Math.round((h * p) / 100);
export const SAFE = {title: [15, 26], tall: [28, 66], caption: [67, 72],
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

const Caption: React.FC<{spec: RenderSpec; height: number}> = ({spec, height}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const t = frame / fps;
  const cap = spec.captions.find((c) => t >= c.start && t < c.end);
  if (!cap) return null;
  // 한 색으로 2~4단어를 통째로 보여 준다 (글자별 색 바뀜·확대 없음)
  return (
    <div style={{position: 'absolute', left: 40, right: 40, top: pct(height, SAFE.caption[0]),
      height: pct(height, SAFE.caption[1] - SAFE.caption[0]), display: 'flex', alignItems: 'center',
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

const TextPage: React.FC<{spec: RenderSpec; text: string; head: boolean; view: number; start: number;
  end: number}> = ({spec, text, head, view, start, end}) => {
  const k = 1080 / SKIN_W;
  const y = useScroll((head ? 52 : 0) + textHeight(text), view / k, start, end);
  return (
    <Scaled width={1080} height={view} scrollY={y}>
      {head ? <PostHead post={spec.post} /> : null}
      <BodyText text={text} />
    </Scaled>
  );
};

const SegmentStage: React.FC<{spec: RenderSpec; seg: SpecSegment; next: SpecSegment | null; height: number}> = ({
  spec, seg, next, height,
}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const t = frame / fps;
  const box = {top: pct(height, SAFE.tall[0]), h: pct(height, SAFE.tall[1] - SAFE.tall[0])};
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
        <Img src={src(seg.image)} style={{maxWidth: '100%', maxHeight: '100%', objectFit: 'contain'}} />
      </div>
    ) : null);
  }
  if (seg.kind === 'body') {
    return wrap(<TextPage spec={spec} text={seg.quote} head={false} view={box.h} start={seg.start} end={seg.end} />);
  }
  // 제목 장면: 글 머리 + 다음 장면 첫 사진 (실제 글 첫 화면처럼)
  const items = seg.kind === 'media' ? seg.media
    : (next?.media.length ? [{...next.media[0], start: seg.start, end: seg.end, muted: true}] : []);
  const m = current(items, t);
  if (!m) {
    return wrap(<TextPage spec={spec} text="" head view={box.h} start={seg.start} end={seg.end} />);
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
  const speaking = (f: number) => spec.scenes.some((s) => f / fps >= s.start && f / fps <= s.end);
  return (
    <AbsoluteFill style={{fontFamily: FONT_FAMILY, background: '#000'}}>
      {seg ? <SegmentStage spec={spec} seg={seg} next={next} height={height} /> : null}
      <Title text={spec.title} height={height} />
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
