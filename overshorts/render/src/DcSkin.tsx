// 디시 화면처럼 다시 그린 본문 글 카드·댓글창 (기획서 3.10). 캡처 대신 수집한 데이터로 그린다.
import React from 'react';
import {Img, interpolate, spring, staticFile, useCurrentFrame, useVideoConfig} from 'remotion';
import {SpecComment} from './spec';

const DC_BLUE = '#3b4890';
const src = (p: string) => (/^(https?:)?\/\//.test(p) || p.startsWith('/') ? p : staticFile(p));

// 글자 수로 줄 수를 어림한다 (넘치면 위로 스크롤할 양 계산용)
const lines = (text: string, perLine: number) =>
  text.split('\n').reduce((n, p) => n + Math.max(1, Math.ceil(p.length / perLine)), 0);

const COMMENT_FONT = 34;
const NICK_W = 250;

export function commentHeight(c: SpecComment): number {
  const perLine = Math.floor((1080 - 48 - NICK_W - (c.depth ? 52 : 0) - 40) / (COMMENT_FONT * 0.95));
  const textH = c.text ? lines(c.text, perLine) * COMMENT_FONT * 1.45 : 0;
  const conH = c.dccon ? 230 : 0;
  return Math.max(textH + conH, COMMENT_FONT * 1.45) + 36;
}

const Mention: React.FC<{text: string}> = ({text}) => (
  <>
    {text.split(/(@\S+)/).map((part, i) => (part.startsWith('@')
      ? <span key={i} style={{color: DC_BLUE}}>{part}</span> : <span key={i}>{part}</span>))}
  </>
);

const NickIcon: React.FC<{icon: SpecComment['icon']}> = ({icon}) => (icon ? (
  <span style={{
    display: 'inline-block', width: 22, height: 22, marginLeft: 6, borderRadius: 4, verticalAlign: -2,
    background: icon === 'fix' ? '#5a6bd1' : '#b7c0ea', border: '1px solid rgba(0,0,0,0.15)',
  }} />
) : null);

const CommentRow: React.FC<{c: SpecComment; blurNick: boolean; reading: boolean}> = ({c, blurNick, reading}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const pop = spring({frame: frame - Math.round(c.appear * fps), fps, config: {damping: 200}, durationInFrames: 10});
  return (
    <div style={{
      display: 'flex', padding: '18px 20px', borderBottom: '1px solid #e6e6e6',
      marginLeft: c.depth ? 52 : 0, background: c.depth ? '#f6f6f6' : '#fff',
      outline: reading ? '5px solid #FFD43B' : 'none', outlineOffset: -5,
      opacity: pop, transform: `translateY(${interpolate(pop, [0, 1], [30, 0])}px)`,
    }}>
      <div style={{width: NICK_W, flexShrink: 0, fontSize: 30, color: '#555', lineHeight: 1.5,
        filter: blurNick ? 'blur(9px)' : undefined}}>
        {c.depth ? <span style={{color: '#999', marginRight: 6}}>└</span> : null}
        <span style={{fontWeight: 600}}>{c.nick}</span>
        {c.ip ? <span style={{color: '#888'}}>({c.ip})</span> : null}
        <NickIcon icon={c.icon} />
      </div>
      <div style={{flex: 1, fontSize: COMMENT_FONT, color: '#222', lineHeight: 1.45, whiteSpace: 'pre-wrap',
        wordBreak: 'break-all'}}>
        {c.text ? <Mention text={c.text} /> : null}
        {c.dccon ? (
          <div style={{marginTop: c.text ? 10 : 0}}>
            {c.dccon.src ? <Img src={src(c.dccon.src)} style={{width: 150, height: 150, objectFit: 'contain'}} />
              : <div style={{width: 150, height: 150, background: '#eee', borderRadius: 8}} />}
            <div style={{display: 'flex', gap: 10, alignItems: 'center', marginTop: 6, fontSize: 24, color: '#777'}}>
              <span>{c.dccon.name}</span>
              <span style={{border: '1px solid #ccc', borderRadius: 4, padding: '1px 8px', fontSize: 22}}>디시콘 보기</span>
            </div>
          </div>
        ) : null}
      </div>
    </div>
  );
};

/** 댓글이 하나씩 아래로 쌓이고 넘치면 위로 스크롤된다. TTS 가 읽는 댓글은 강조 테두리. */
export const CommentBox: React.FC<{comments: SpecComment[]; height: number; blurNick: boolean}> = ({
  comments, height, blurNick,
}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const t = frame / fps;
  const shown = comments.filter((c) => t + 0.2 >= c.appear);
  const total = shown.reduce((h, c) => h + commentHeight(c), 0) + 60;
  const target = Math.max(0, total - height);
  const last = shown[shown.length - 1];
  const ease = last ? spring({frame: frame - Math.round(last.appear * fps), fps, config: {damping: 200},
    durationInFrames: 14}) : 1;
  const prev = Math.max(0, total - (last ? commentHeight(last) : 0) - height);
  const scroll = interpolate(ease, [0, 1], [prev, target]);
  return (
    <div style={{position: 'absolute', inset: 0, background: '#fff', overflow: 'hidden', borderRadius: 18}}>
      <div style={{height: 6, background: DC_BLUE}} />
      <div style={{padding: '12px 22px', fontSize: 28, color: '#333', borderBottom: '2px solid ' + DC_BLUE,
        fontWeight: 700}}>
        전체 댓글 <span style={{color: '#d31900'}}>{comments.length}</span>개
      </div>
      <div style={{transform: `translateY(${-scroll}px)`}}>
        {shown.map((c) => (
          <CommentRow key={c.no} c={c} blurNick={blurNick}
            reading={!!c.read && t >= c.read[0] && t <= c.read[1]} />
        ))}
      </div>
    </div>
  );
};

/** 디시 본문처럼 그린 글 카드. 글이 길면 세그먼트 동안 천천히 아래로. */
export const BodyCard: React.FC<{title: string; subject: string; text: string; height: number;
  start: number; end: number}> = ({title, subject, text, height, start, end}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const t = frame / fps;
  const textH = lines(text, 25) * 40 * 1.65 + 200;
  const over = Math.max(0, textH - height);
  const y = over ? interpolate(t, [start + 1.2, Math.max(start + 1.3, end - 0.5)], [0, over],
    {extrapolateLeft: 'clamp', extrapolateRight: 'clamp'}) : 0;
  return (
    <div style={{position: 'absolute', inset: 0, background: '#fff', overflow: 'hidden', borderRadius: 18}}>
      <div style={{height: 6, background: DC_BLUE}} />
      <div style={{transform: `translateY(${-y}px)`}}>
        <div style={{padding: '22px 30px 16px', borderBottom: '1px solid #ddd'}}>
          <div style={{fontSize: 38, fontWeight: 700, color: '#222', lineHeight: 1.3, wordBreak: 'keep-all'}}>
            {subject ? <span style={{color: '#666', fontWeight: 500}}>[{subject}] </span> : null}{title}
          </div>
        </div>
        <div style={{padding: '24px 30px', fontSize: 40, lineHeight: 1.65, color: '#222', whiteSpace: 'pre-wrap',
          wordBreak: 'break-all'}}>{text}</div>
      </div>
    </div>
  );
};
