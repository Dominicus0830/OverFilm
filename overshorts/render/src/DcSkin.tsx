// 디시 화면 스크린샷처럼 다시 그린 글 머리·사진·본문·댓글창 (기획서 3.10, 2026-11-15 사용자 피드백).
// 실제 디시 크기(가로 480px, 글자 13px)로 그린 뒤 화면 폭에 맞게 통째로 키운다. 연출(확대·글자별 색) 없음.
import React from 'react';
import {Img, OffthreadVideo, Sequence, interpolate, staticFile, useCurrentFrame, useVideoConfig} from 'remotion';
import {GALLERCON_SVG, REPLY_ARROW_PNG} from './dcAssets';
import {mapFont} from './fonts';
import {SpecComment, SpecMedia, SpecPost, StyledLine, StyledRun} from './spec';

export const SKIN_W = 480;
const DC_BLUE = '#3b4890';
const FONT = 13;
const src = (p: string) => (/^(https?:)?\/\//.test(p) || p.startsWith('/') ? p : staticFile(p));

/** 원문 글자 크기(px, 디시 PC 화면 기준) → 480px 화면 크기. 기본 크기는 그대로 13. */
export function skinSize(px: number | undefined): number {
  if (!px || px <= 16) return FONT;
  return Math.min(30, Math.round(px * 0.65));
}

/** 꾸밈 조각 → CSS (색·배경·글꼴·굵게·기울임·밑줄·취소선). 크기는 부르는 쪽이 정한다. */
export function runCss(r: StyledRun): React.CSSProperties {
  const deco = [r.underline ? 'underline' : '', r.strike ? 'line-through' : ''].filter(Boolean).join(' ');
  return {
    color: r.color, background: r.bg, fontFamily: mapFont(r.font),
    fontWeight: r.bold ? 800 : undefined, fontStyle: r.italic ? 'italic' : undefined,
    textDecoration: deco || undefined,
  };
}

/** 원문 꾸밈을 살린 여러 줄 (디시 화면 속 글). */
export const RichLines: React.FC<{lines: StyledLine[]}> = ({lines}) => (
  <>
    {lines.map((line, i) => {
      const size = Math.max(...line.map((r) => skinSize(r.size)), FONT);
      return (
        <div key={i} style={{fontSize: size, lineHeight: 1.5, minHeight: size * 1.5}}>
          {line.map((r, k) => <span key={k} style={{...runCss(r), fontSize: skinSize(r.size)}}>{r.t}</span>)}
        </div>
      );
    })}
  </>
);

/** 꾸민 줄들의 높이 (가로 468px 에 한글 한 글자 ≈ 글자 크기). */
export function richHeight(lines: StyledLine[]): number {
  return lines.reduce((h, line) => {
    const size = Math.max(...line.map((r) => skinSize(r.size)), FONT);
    const width = line.reduce((w, r) => w + r.t.length * skinSize(r.size), 0);
    return h + Math.max(1, Math.ceil(width / (SKIN_W - 12))) * size * 1.5;
  }, 0);
}

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

// 닉 옆 딱지: 디시 sp_gallercon.svg 스프라이트 (고닉 fix_nik / 반고닉 nik 자리 그대로).
const NickIcon: React.FC<{icon: string}> = ({icon}) => (icon ? (
  <span style={{display: 'inline-block', boxSizing: 'content-box', width: 0, height: 0, padding: '11px 0 0 12px',
    marginLeft: 2, verticalAlign: -1, background: `url(${GALLERCON_SVG}) no-repeat`,
    backgroundSize: icon === 'fix' ? 101 : 92, backgroundPosition: icon === 'fix' ? '-0.5px -17px' : '0 0'}} />
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

/** 본문 속 사진·영상과 그 아래 본문 줄. 번호 칩·'댓글 열기' 버튼은 안 그린다 (2026-11-21 사용자). */
export const PhotoBlock: React.FC<{m: SpecMedia}> = ({m}) => {
  const {fps} = useVideoConfig();
  const aspect = m.width && m.height ? m.width / m.height : 16 / 9;
  const h = Math.round(SKIN_W / aspect);
  const style: React.CSSProperties = {width: SKIN_W, height: h, objectFit: 'cover', display: 'block'};
  return (
    <div style={{padding: '8px 0 0'}}>
      <div style={{position: 'relative'}}>
        {m.kind === 'video' ? (
          // 영상은 장면이 시작할 때 처음부터 (Sequence 없이는 영상 전체 시각으로 재생돼 중간부터 나오고 일찍 끝나 멈춰 있었다)
          <Sequence from={Math.round(m.start * fps)} layout="none">
            <OffthreadVideo src={src(m.src)} muted={m.muted} style={style}
              playbackRate={m.playback_rate && m.playback_rate > 0 ? m.playback_rate : 1} />
          </Sequence>
        ) : <Img src={src(m.src)} style={style} />}
      </div>
      {m.caption_runs ? (
        <div style={{padding: '4px 2px 6px', wordBreak: 'break-all'}}><RichLines lines={m.caption_runs} /></div>
      ) : m.caption ? (
        <div style={{padding: '4px 2px 6px', whiteSpace: 'pre-wrap', wordBreak: 'break-all'}}>{m.caption}</div>
      ) : null}
    </div>
  );
};

export function photoHeight(m: SpecMedia): number {
  const aspect = m.width && m.height ? m.width / m.height : 16 / 9;
  if (m.caption_runs) return 8 + SKIN_W / aspect + richHeight(m.caption_runs) + 10;
  const lines = m.caption ? m.caption.split('\n').reduce((n, p) => n + Math.max(1, Math.ceil(p.length / 36)), 0) : 0;
  return 8 + SKIN_W / aspect + lines * FONT * 1.5 + 10;
}

// 디시 댓글창: 실제 디시 PC 댓글 HTML 구조와 CSS(common.css)를 그대로 쓴다 (2026-11-21 사용자: "HTML CSS 그대로").
// 폭만 좁힌 가상 화면(CMT_W)에 그리고, 내용 높이만큼만 흰 상자로 화면 가운데에 놓는다.
export const CMT_W = 560;
const DATE_W = 86; // '11.14 17:45:31' + 여백
const TXT_W = CMT_W - 24 - (152 + 33) - DATE_W; // 원댓글 글 칸 (한 줄 약 20자)
const R_TXT_W = CMT_W - 30 - 2 - 24 - (133 + 23) - DATE_W - 16; // 답글 글 칸 (약 18자)
const DC_FONT = '-apple-system,BlinkMacSystemFont,"Apple SD Gothic Neo","Malgun Gothic","맑은 고딕",arial,굴림,Gulim,sans-serif';

const DC_CSS = `
.dcc{width:${CMT_W}px;background:#fff;color:#333;font-size:13px;font-family:${DC_FONT};border-top:2px solid #3b4890;border-bottom:1px solid #eee;text-align:left}
.dcc ul,.dcc li,.dcc p{margin:0;padding:0;list-style:none}
.dcc em{font-style:normal}
.dcc .clear:after{clear:both;display:block;visibility:hidden;content:""}
.dcc .fr{float:right}
.dcc .cmt_info{position:relative;padding:9px 12px 7px;border-top:1px solid #eee}
.dcc .cmt_list>li:first-child .cmt_info{border-top:none}
.dcc .cmt_info p{line-height:20px}
.dcc .cmt_nickbox{float:left;width:152px;margin-right:33px;margin-top:3px;line-height:13px}
.dcc .gall_writer{display:inline-flex;align-items:start;max-width:100%}
.dcc .nickname{font-size:12px;color:#777;vertical-align:top;white-space:nowrap}
.dcc .nickname>em{display:inline-block;max-width:84px;text-overflow:ellipsis;overflow:hidden;white-space:nowrap;vertical-align:top}
.dcc .nickname.in>em{max-width:110px;padding-right:1px}
.dcc .reply .nickname.in>em{max-width:107px}
.dcc .nickname .ip{display:inline-block;vertical-align:1px;font-family:tahoma,sans-serif;font-size:11px;color:#999}
.dcc .nomem_nicknum{color:#333}
.dcc .writer_nikcon{margin-left:2px;line-height:0}
.dcc .writer_nikcon i{display:inline-block;box-sizing:content-box;width:0;height:0;padding:11px 0 0 12px;vertical-align:-1px;background:url(${GALLERCON_SVG}) no-repeat;background-size:92px;background-position:0 0}
.dcc .writer_nikcon i.fix{background-size:101px;background-position:-0.5px -17px}
.dcc .cmt_txtbox{float:left;width:${TXT_W}px}
.dcc .usertxt{float:left;width:${TXT_W}px;line-height:20px;word-break:break-all;overflow:hidden;white-space:pre-wrap}
.dcc .date_time{float:left;font-size:12px;color:#999;vertical-align:top;margin-top:1px}
.dcc .reply{overflow:visible}
.dcc .cmt_list>li:first-child .reply{padding:12px 0 0;border-top:none}
.dcc .reply_box{margin:0 0 12px 30px;border:1px solid #ddd;border-width:0 1px 1px 1px;background:#fafafa}
.dcc .cmt_list>li:first-child .reply_box{border-top-width:1px}
.dcc .reply_list li{padding:9px 12px 7px;border-top:1px solid #ddd}
.dcc .reply_list li:first-child{border-top:none}
.dcc .reply .cmt_nickbox{width:133px;margin-right:23px}
.dcc .reply .cmt_txtbox{width:${R_TXT_W + 16}px}
.dcc .reply_info{position:relative}
.dcc .reply_info .usertxt{position:relative;width:${R_TXT_W}px;padding-left:16px}
.dcc .reply_info .usertxt::before{content:" ";position:absolute;left:0;top:3px;width:8px;height:8px;background:url(${REPLY_ARROW_PNG}) no-repeat}
.dcc .comment_dccon{position:relative;margin:3px 0 5px;float:left}
.dcc .reply .comment_dccon{padding-left:16px}
.dcc .coment_dccon_img{float:left;height:100px;line-height:0}
.dcc .written_dccon{width:100px;height:100px;object-fit:contain}
.dcc .blur{filter:blur(4px)}
`;

/** 닉네임 칸: 디시 gallog_icon 그대로 (고닉 fix_nik, 반고닉 nik, 유동은 (IP)). */
const Writer: React.FC<{c: SpecComment}> = ({c}) => (
  <span className="gall_writer ub-writer">
    <span className={c.icon ? 'nickname in' : 'nickname'}><em>{c.nick}</em>
      {!c.icon && c.ip ? <span className="ip">({c.ip})</span> : null}</span>
    {c.icon ? <a className="writer_nikcon"><i className={c.icon === 'fix' ? 'fix' : ''} /></a> : null}
  </span>
);

/** 댓글의 디시콘 그림들 (받은 그림만. 못 받은 건 괄호 글자 대신 안 보임). */
const cons = (c: SpecComment): string[] => (c.dccons?.length ? c.dccons.map((d) => d.src)
  : c.dccon?.src ? [c.dccon.src] : []);

const CmtBody: React.FC<{c: SpecComment; blurNick: boolean}> = ({c, blurNick}) => (
  <>
    <div className={blurNick ? 'cmt_nickbox blur' : 'cmt_nickbox'}><Writer c={c} /></div>
    <div className="clear cmt_txtbox">
      {c.text ? <p className="usertxt ub-word">{c.text}</p> : null}
      {cons(c).length ? (
        <div className="comment_dccon clear">
          {cons(c).map((d, i) => (
            <div key={i} className="coment_dccon_img"><Img className="written_dccon" src={src(d)} /></div>
          ))}
        </div>
      ) : null}
    </div>
    <div className="fr clear">{c.date ? <span className="date_time">{c.date}</span> : null}</div>
  </>
);

/** 댓글창 한 장 (디시 화면을 그대로 캡처한 것처럼). 원댓글 밑 답글은 reply_box 안에. */
export const CommentPage: React.FC<{comments: SpecComment[]; blurNick: boolean}> = ({comments, blurNick}) => {
  const blocks: SpecComment[][] = [];
  for (const c of comments) {
    if (c.depth && blocks.length && blocks[blocks.length - 1][0].depth) blocks[blocks.length - 1].push(c);
    else blocks.push([c]);
  }
  return (
    <div className="dcc comment_box">
      <style>{DC_CSS}</style>
      <ul className="cmt_list">
        {blocks.map((b) => (b[0].depth ? (
          <li key={`r${b[0].no}`}>
            <div className="reply show"><div className="reply_box"><ul className="reply_list">
              {b.map((c) => <li key={c.no} className="ub-content"><div className="reply_info clear">
                <CmtBody c={c} blurNick={blurNick} /></div></li>)}
            </ul></div></div>
          </li>
        ) : (
          <li key={b[0].no} className="ub-content"><div className="cmt_info clear">
            <CmtBody c={b[0]} blurNick={blurNick} /></div></li>
        )))}
      </ul>
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
export const BodyText: React.FC<{text: string; runs?: StyledLine[] | null}> = ({text, runs}) => (runs ? (
  <div style={{padding: '10px 6px', wordBreak: 'keep-all', overflowWrap: 'anywhere'}}><RichLines lines={runs} /></div>
) : (
  <div style={{padding: '10px 6px', whiteSpace: 'pre-wrap', wordBreak: 'keep-all', overflowWrap: 'anywhere'}}>{text}</div>
));

export function textHeight(text: string, runs?: StyledLine[] | null): number {
  if (runs) return 20 + richHeight(runs);
  return 20 + text.split('\n').reduce((n, p) => n + Math.max(1, Math.ceil(p.length / 36)), 0) * FONT * 1.5;
}
