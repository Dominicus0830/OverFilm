// 썰 카드형: 상단 제목바 + 중앙 글 카드 + 하단 배경 영상, 단어 팝업 자막, 핵심 단어 색 강조, 출처 표기.
import React from 'react';
import {
  AbsoluteFill, Audio, Loop, OffthreadVideo, interpolate, spring, staticFile, useCurrentFrame,
  useVideoConfig,
} from 'remotion';
import {FONT_FAMILY, loadFonts} from './fonts';
import {RenderSpec, SpecScene, sanitize} from './spec';

loadFonts();

const src = (p: string) => (/^https?:\/\//.test(p) ? p : staticFile(p));

function useScene(spec: RenderSpec, t: number): SpecScene | null {
  let current: SpecScene | null = null;
  for (const sc of spec.scenes) {
    if (sc.start <= t + 0.05) current = sc;
  }
  return current;
}

const TitleBar: React.FC<{spec: RenderSpec}> = ({spec}) => (
  <div style={{
    position: 'absolute', top: 0, left: 0, right: 0, height: 260, padding: '110px 64px 0',
    background: spec.theme.dark ? '#0d0f12' : '#ffffff',
    color: spec.theme.dark ? '#ffffff' : '#111',
    fontSize: 64, fontWeight: 800, letterSpacing: -1.5, lineHeight: 1.15,
    whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
  }}>
    {spec.title}
  </div>
);

const PostCard: React.FC<{spec: RenderSpec; scene: SpecScene | null}> = ({spec, scene}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const t = frame / fps;
  const isComment = scene?.speaker === 'comment';
  const appear = scene ? spring({frame: Math.max(0, frame - Math.round(scene.start * fps)), fps,
    config: {damping: 200}, durationInFrames: 12}) : 1;
  const dark = spec.theme.dark;
  return (
    <div style={{
      position: 'absolute', top: 320, left: 56, right: 56, minHeight: 560,
      borderRadius: 36, padding: '48px 52px',
      background: isComment ? (dark ? '#1f2a37' : '#eef3fb') : (dark ? '#1a1d22' : '#f6f6f4'),
      color: dark ? '#f2f2f2' : '#1a1a1a',
      boxShadow: '0 20px 60px rgba(0,0,0,0.35)',
      transform: `translateY(${interpolate(appear, [0, 1], [24, 0])}px)`,
      opacity: interpolate(appear, [0, 1], [0.6, 1]),
    }}>
      <div style={{display: 'flex', gap: 16, alignItems: 'center', marginBottom: 28}}>
        <span style={{
          fontSize: 34, fontWeight: 700, padding: '6px 18px', borderRadius: 999,
          background: isComment ? '#3b82f6' : spec.theme.accent, color: isComment ? '#fff' : '#111',
        }}>{isComment ? '댓글' : (spec.subject || '글')}</span>
      </div>
      <div style={{fontSize: 58, fontWeight: 700, lineHeight: 1.38, letterSpacing: -1, wordBreak: 'keep-all'}}>
        {scene?.words.length ? scene.words.map((w, i) => (
          <span key={i} style={{
            color: w.emphasis ? spec.theme.accent : undefined,
            opacity: t + 0.02 >= w.start ? 1 : 0.28,
          }}>{w.text} </span>
        )) : scene?.text}
      </div>
    </div>
  );
};

const Captions: React.FC<{spec: RenderSpec; scene: SpecScene | null}> = ({spec, scene}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const t = frame / fps;
  if (!scene) return null;
  // 지금 읽는 단어 하나를 크게 띄운다 (단어 단위 팝업)
  const idx = scene.words.findIndex((w) => t >= w.start && t < Math.max(w.end, w.start + 0.12));
  const word = idx >= 0 ? scene.words[idx] : null;
  if (!word) return null;
  const pop = spring({frame: frame - Math.round(word.start * fps), fps, config: {damping: 12, stiffness: 220},
    durationInFrames: 10});
  return (
    <div style={{position: 'absolute', left: 0, right: 0, top: 1380, textAlign: 'center'}}>
      <span style={{
        display: 'inline-block', fontSize: 104, fontWeight: 900, letterSpacing: -2,
        color: word.emphasis ? spec.theme.accent : '#ffffff',
        WebkitTextStroke: '10px #000', paintOrder: 'stroke fill',
        transform: `scale(${interpolate(pop, [0, 1], [0.7, 1])})`,
      }}>{word.text}</span>
    </div>
  );
};

export const CardStory: React.FC<Record<string, unknown>> = (props) => {
  const spec = sanitize(props);
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const scene = useScene(spec, frame / fps);
  return (
    <AbsoluteFill style={{fontFamily: FONT_FAMILY, background: spec.theme.dark ? '#0d0f12' : '#fff'}}>
      <AbsoluteFill style={{top: 960}}>
        {spec.background ? (
          <Loop durationInFrames={Math.max(1, Math.round(spec.duration * fps))}>
            <OffthreadVideo src={src(spec.background)} muted
              style={{width: '100%', height: '100%', objectFit: 'cover'}} />
          </Loop>
        ) : (
          <AbsoluteFill style={{background: 'linear-gradient(180deg, #1b2330 0%, #0b0d10 100%)'}} />
        )}
      </AbsoluteFill>
      <TitleBar spec={spec} />
      <PostCard spec={spec} scene={scene} />
      <Captions spec={spec} scene={scene} />
      <div style={{position: 'absolute', bottom: 70, left: 0, right: 0, textAlign: 'center',
        fontSize: 30, color: 'rgba(255,255,255,0.75)'}}>{spec.source}</div>
      {spec.audio ? <Audio src={src(spec.audio)} /> : null}
    </AbsoluteFill>
  );
};
