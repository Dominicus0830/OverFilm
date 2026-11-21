import React, {useRef} from 'react';
import type {RenderSpec} from '../../../render/src/spec';

const SEG_KO: Record<string, string> = {title: '제목', media: '본문 미디어', body: '본문 글', comments: '댓글 반응', reaction: '짤'};

/**
 * 아래 '타임라인' 탭: 미리보기 스펙의 장면(세그먼트)·자막·음성 줄을 시간 축에 놓는다.
 * 장면을 누르면 그 장면 처음으로 가고 오른쪽 속성의 같은 장면 대본으로 스크롤, 눈금을 누르면 그 시각으로 간다.
 */
export function Timeline({spec, time, seg, onSeek, onPickSeg}: {
  spec: RenderSpec; time: number; seg: number | null; onSeek: (t: number) => void; onPickSeg: (i: number) => void;
}) {
  const total = Math.max(0.1, spec.duration);
  const pct = (t: number) => `${(Math.min(total, Math.max(0, t)) / total) * 100}%`;
  const ruler = useRef<HTMLDivElement>(null);
  const step = total > 90 ? 10 : 5;
  const seekAt = (clientX: number) => {
    const r = ruler.current?.getBoundingClientRect();
    if (!r || r.width <= 0) return;
    onSeek(((clientX - r.left) / r.width) * total);
  };
  const narr = spec.scenes.filter((s) => s.speaker === 'narrator');
  const cmt = spec.scenes.filter((s) => s.speaker === 'comment');

  return (
    <div className="tl" data-automation-id="timeline">
      <div className="tl-row tl-ruler">
        <span className="tl-name" />
        <div className="tl-track ruler" ref={ruler} onPointerDown={(e) => seekAt(e.clientX)}
          role="slider" aria-label="재생 위치" aria-valuemin={0} aria-valuemax={Math.round(total)} aria-valuenow={Math.round(time)}
          tabIndex={0} onKeyDown={(e) => {
            if (e.key === 'ArrowLeft') onSeek(time - 1); else if (e.key === 'ArrowRight') onSeek(time + 1); else return;
            e.preventDefault();
          }}>
          {Array.from({length: Math.floor(total / step) + 1}, (_, i) => (
            <span key={i} className="tick num" style={{left: pct(i * step)}}>{i * step}초</span>
          ))}
        </div>
      </div>
      <div className="tl-row tall">
        <span className="tl-name">장면</span>
        <div className="tl-track abs">
          {spec.segments.map((g, i) => (
            <button key={i} className={i === seg ? 'clip on' : 'clip'} style={{left: pct(g.start), width: `calc(${pct(g.end - g.start)} - 2px)`}}
              onClick={() => onPickSeg(i)} title={`${SEG_KO[g.kind] ?? g.kind} ${(g.end - g.start).toFixed(1)}초`}
              data-automation-id={`scene-${i}`}>
              <span className="clip-name">{i + 1}. {SEG_KO[g.kind] ?? g.kind}</span>
              <span className="clip-len num">{(g.end - g.start).toFixed(1)}초{g.tts ? '' : ' · 소리 없음'}</span>
            </button>
          ))}
          {spec.segments.length === 0 && <span className="faint small pad">장면 정보가 없는 옛 미리보기입니다</span>}
        </div>
      </div>
      <div className="tl-row">
        <span className="tl-name">자막</span>
        <div className="tl-track abs">
          {spec.captions.map((c, i) => (
            <span key={i} className="sub" style={{left: pct(c.start), width: `calc(${pct(c.end - c.start)} - 1px)`}}
              title={c.words.map((w) => w.text).join(' ')} onClick={() => onSeek(c.start)} />
          ))}
        </div>
      </div>
      <div className="tl-row">
        <span className="tl-name">나레이션</span>
        <div className="tl-track abs">
          {narr.map((s, i) => (
            <span key={i} className="voice" style={{left: pct(s.start), width: `calc(${pct(s.end - s.start)} - 1px)`}}
              title={s.text} onClick={() => onSeek(s.start)} />
          ))}
        </div>
      </div>
      {cmt.length > 0 && (
        <div className="tl-row">
          <span className="tl-name">댓글 읽기</span>
          <div className="tl-track abs">
            {cmt.map((s, i) => (
              <span key={i} className="voice alt" style={{left: pct(s.start), width: `calc(${pct(s.end - s.start)} - 1px)`}}
                title={s.text} onClick={() => onSeek(s.start)} />
            ))}
          </div>
        </div>
      )}
      <div className="tl-row">
        <span className="tl-name">배경음</span>
        <div className="tl-track"><span className="bgm">{spec.bgm ? spec.bgm.split('/').pop() : '없음'}</span></div>
      </div>
      <div className="playhead-lane" aria-hidden="true"><span className="playhead" style={{left: pct(time)}} /></div>
    </div>
  );
}
