// 디시 화면 스크린샷처럼 다시 그린 글 머리·사진·본문·댓글창 (기획서 3.10, 2026-11-15 사용자 피드백).
// 실제 디시 크기(가로 480px, 글자 13px)로 그린 뒤 화면 폭에 맞게 통째로 키운다. 연출(확대·글자별 색) 없음.
import React from 'react';
import {Img, OffthreadVideo, interpolate, staticFile, useCurrentFrame, useVideoConfig} from 'remotion';
import {SpecComment, SpecMedia, SpecPost} from './spec';

export const SKIN_W = 480;
const DC_BLUE = '#3b4890';
const FONT = 13;
const src = (p: string) => (/^(https?:)?\/\//.test(p) || p.startsWith('/') ? p : staticFile(p));

/** 가상 480px 화면을 width(실제 px)에 맞게 키운다. */
export const Scaled: React.FC<{width: number; height: number; children: React.ReactNode; scrollY?: number}> = ({
  width, height, children, scrollY = 0,
}) => {
  const k = width / SKIN_W;
  return (
    <div style={{position: 'absolute', inset: 0, overflow: 'hidden', background: '#fff'}}>
      <div style={{width: SKIN_W, height: height / k, transform: `scale(${k})`, transformOrigin: 'top left'}}>
        <div style={{transform: `translateY(${-scrollY}px)`, color: '#333', fontSize: FONT, lineHeight: 1.5}}>
          {children}
        </div>
      </div>
    </div>
  );
};

const NickIcon: React.FC<{icon: string}> = ({icon}) => (icon ? (
  <span style={{display: 'inline-block', width: 12, height: 11, marginLeft: 3, verticalAlign: -1, borderRadius: 2,
    background: icon === 'fix' ? '#6b79c8' : '#c4c9de', border: '1px solid #9aa'}} />
) : null);

/** 글 머리: [말머리] 제목 / 닉 · 날짜 */
export const PostHead: React.FC<{post: SpecPost}> = ({post}) => (
  <div style={{padding: '10px 10px 8px', borderBottom: '1px solid #ddd', borderTop: `2px solid ${DC_BLUE}`}}>
    <div style={{fontSize: 15, fontWeight: 700, color: '#333', lineHeight: 1.4, wordBreak: 'keep-all'}}>
      {post.subject ? `[${post.subject}] ` : ''}{post.title}
    </div>
    <div style={{fontSize: 12, color: '#777', marginTop: 4}}>
      {post.nick}{post.ip ? `(${post.ip})` : ''}<NickIcon icon={post.icon} />
      {post.date ? <span style={{marginLeft: 8, paddingLeft: 8, borderLeft: '1px solid #ccc'}}>{post.date}</span> : null}
    </div>
  </div>
);

/** 본문 속 사진: 오른쪽 위 번호, 오른쪽 아래 '댓글 열기', 아래에 그 사진 다음 본문 줄 */
export const PhotoBlock: React.FC<{m: SpecMedia}> = ({m}) => {
  const aspect = m.width && m.height ? m.width / m.height : 16 / 9;
  const h = Math.round(SKIN_W / aspect);
  const style: React.CSSProperties = {width: SKIN_W, height: h, objectFit: 'cover', display: 'block'};
  return (
    <div style={{padding: '8px 0 0'}}>
      <div style={{position: 'relative'}}>
        {m.kind === 'video' ? <OffthreadVideo src={src(m.src)} muted={m.muted} style={style} />
          : <Img src={src(m.src)} style={style} />}
        {m.number ? (
          <span style={{position: 'absolute', top: 6, right: 6, minWidth: 16, height: 16, fontSize: 11,
            lineHeight: '16px', textAlign: 'center', background: '#fff', color: '#555', border: '1px solid #ccc',
            fontWeight: 700}}>{m.number}</span>
        ) : null}
        <span style={{position: 'absolute', right: 8, bottom: 8, fontSize: 11, color: '#fff', padding: '4px 9px',
          background: 'rgba(40,40,40,0.85)', borderRadius: 3, fontWeight: 700}}>댓글 열기 ▾</span>
      </div>
      {m.caption ? (
        <div style={{padding: '4px 2px 6px', whiteSpace: 'pre-wrap', wordBreak: 'break-all'}}>{m.caption}</div>
      ) : null}
    </div>
  );
};

export function photoHeight(m: SpecMedia): number {
  const aspect = m.width && m.height ? m.width / m.height : 16 / 9;
  const lines = m.caption ? m.caption.split('\n').reduce((n, p) => n + Math.max(1, Math.ceil(p.length / 36)), 0) : 0;
  return 8 + SKIN_W / aspect + lines * FONT * 1.5 + 10;
}

const CommentRow: React.FC<{c: SpecComment; blurNick: boolean}> = ({c, blurNick}) => (
  <div style={{display: 'flex', padding: '7px 8px', borderBottom: '1px solid #eee'}}>
    <div style={{width: c.depth ? 98 : 118, flexShrink: 0, fontSize: 12, color: '#555', paddingLeft: c.depth ? 0 : 0,
      filter: blurNick ? 'blur(4px)' : undefined, whiteSpace: 'nowrap', overflow: 'hidden'}}>
      {c.nick}{c.ip ? <span style={{color: '#999'}}>({c.ip})</span> : null}<NickIcon icon={c.icon} />
    </div>
    <div style={{flex: 1, display: 'flex', gap: 6}}>
      {c.depth ? <span style={{color: '#8a94c4', fontSize: 12}}>ㄴ</span> : null}
      <div style={{flex: 1, whiteSpace: 'pre-wrap', wordBreak: 'break-all'}}>
        {c.text ? c.text.split(/(@\S+)/).map((part, i) => (part.startsWith('@')
          ? <span key={i} style={{color: DC_BLUE}}>{part}</span> : <span key={i}>{part}</span>)) : null}
        {c.dccon ? (
          <div style={{marginTop: c.text ? 4 : 0}}>
            {c.dccon.src ? <Img src={src(c.dccon.src)} style={{width: 90, height: 90, objectFit: 'contain'}} />
              : <span style={{color: '#999'}}>({c.dccon.name})</span>}
          </div>
        ) : null}
      </div>
    </div>
  </div>
);

/** 댓글창 한 장 (스크린샷처럼 한 번에). 원댓글 밑 답글은 회색 상자 안에. */
export const CommentPage: React.FC<{comments: SpecComment[]; blurNick: boolean}> = ({comments, blurNick}) => {
  const blocks: SpecComment[][] = [];
  for (const c of comments) {
    if (c.depth && blocks.length && blocks[blocks.length - 1][0].depth) blocks[blocks.length - 1].push(c);
    else blocks.push([c]);
  }
  return (
    <div style={{borderTop: `2px solid ${DC_BLUE}`}}>
      {blocks.map((b, i) => (b[0].depth ? (
        <div key={i} style={{background: '#f8f8f8', border: '1px solid #e3e3e3', margin: '0 0 0 18px'}}>
          {b.map((c) => <CommentRow key={c.no} c={c} blurNick={blurNick} />)}
        </div>
      ) : <CommentRow key={b[0].no} c={b[0]} blurNick={blurNick} />))}
    </div>
  );
};

/** 사진·글이 화면보다 길면 세그먼트 동안 천천히 아래로 (페이지를 읽어 내려가듯). */
export function useScroll(contentH: number, viewH: number, start: number, end: number): number {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const over = Math.max(0, contentH - viewH);
  if (!over) return 0;
  // 앞부분(글 머리)을 충분히 보여 준 뒤 내려간다
  const from = start + Math.max(1.5, (end - start) * 0.4);
  return interpolate(frame / fps, [from, Math.max(from + 0.1, end - 0.4)], [0, over],
    {extrapolateLeft: 'clamp', extrapolateRight: 'clamp'});
}

/** 본문 글만 있는 장면 (사진 없는 부분). */
export const BodyText: React.FC<{text: string}> = ({text}) => (
  <div style={{padding: '10px 6px', whiteSpace: 'pre-wrap', wordBreak: 'break-all'}}>{text}</div>
);

export function textHeight(text: string): number {
  return 20 + text.split('\n').reduce((n, p) => n + Math.max(1, Math.ceil(p.length / 36)), 0) * FONT * 1.5;
}
