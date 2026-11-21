import React, {useEffect, useState} from 'react';
import {api} from '../api';

// 장면별 글 연출 (2026-11-21 사용자): AI(Claude Code 또는 Ollama)가 정한 글 등장·퇴장 시점과 크기·색·굵기·글꼴을
// 검수 화면에서 보고 고친다. 고치면 미리보기에 바로 반영 (다시 만들지 않음).
type Pick = {id: string; appear: 'rule' | 'start' | 'delay'; delay: number; until: 'rule' | 'end' | 'hold';
  hold: number; size: number; color: string | null; bold: boolean | null; font: 'base' | 'shout' | 'hand' | null;
  reason: string; manual?: boolean};
type Row = {id: string; where: 'caption' | 'media'; seg: number; kind: string; text: string;
  scene: [number, number]; rule: [number, number]; styled?: boolean; pick: Pick};
type View = {on: boolean; source: string; rows: Row[]};

const KIND_KO: Record<string, string> = {title: '제목', media: '사진·영상', body: '본문', comments: '댓글',
  reaction: '짤', image: '사진', video: '영상'};

export function TextDirection({jobId, tick, onSaved}: {jobId: number; tick: number; onSaved: () => void}) {
  const [view, setView] = useState<View | null>(null);
  const [picks, setPicks] = useState<Pick[]>([]);
  const [dirty, setDirty] = useState(false);
  const [msg, setMsg] = useState('');

  useEffect(() => {
    api<View>(`/api/review/${jobId}/direction`).then((v) => {
      setView(v); setPicks(v.rows.map((r) => r.pick)); setDirty(false); setMsg('');
    }).catch((e) => { setView(null); setMsg(e.message); });
  }, [jobId, tick]);

  if (!view) return msg ? <p className="muted small">글 연출: {msg}</p> : null;
  if (!view.rows.length) return null;

  const set = (i: number, patch: Partial<Pick>) => {
    setPicks((ps) => ps.map((p, k) => (k === i ? {...p, ...patch} : p)));
    setDirty(true);
  };
  const save = async (list: Pick[]) => {
    try {
      const got = await api<{rows: Row[]}>(`/api/review/${jobId}/direction`, {picks: list});
      setPicks(got.rows.map((r) => r.pick)); setDirty(false); setMsg('미리보기에 반영했습니다');
      onSaved();
    } catch (e) { setMsg((e as Error).message); }
  };
  const allRule = () => save(view.rows.map((r) => ({...r.pick, appear: 'rule', until: 'rule', size: 1, color: null,
    bold: null, font: null, reason: '검수에서 규칙대로'})));
  const src = view.source === 'manual' ? '검수에서 고친 값' : view.source === 'ai' ? 'AI 판단' : '규칙대로';

  return (
    <details className="card" data-automation-id="review-direction">
      <summary>글 연출 ({src}) · 글 {view.rows.length}개</summary>
      <p className="muted small">글이 언제 뜨고 사라질지, 크기·색·굵기·글꼴. 글 내용은 바꾸지 않습니다.</p>
      {view.rows.map((r, i) => {
        const p = picks[i];
        if (!p) return null;
        return (
          <div key={r.id} className="direction-row" style={{borderTop: '1px solid var(--line, #3333)', padding: '6px 0'}}>
            <div className="small">
              <b>{r.id}</b> · 장면 {r.seg + 1} {KIND_KO[r.kind] ?? r.kind} ·{' '}
              {r.where === 'caption' ? '화면 자막' : '사진·영상 아래 글'} · 장면 {r.scene[0].toFixed(1)}~{r.scene[1].toFixed(1)}초
            </div>
            <div style={{margin: '2px 0 4px'}}>{r.text}</div>
            <div className="small" style={{display: 'flex', flexWrap: 'wrap', gap: 6, alignItems: 'center'}}>
              <label>뜨기 <select value={p.appear} onChange={(e) => set(i, {appear: e.target.value as Pick['appear']})}
                data-automation-id={`dir-appear-${r.id}`}>
                <option value="rule">규칙대로</option><option value="start">장면과 같이</option><option value="delay">몇 초 뒤</option>
              </select></label>
              {p.appear === 'delay' && <label><input type="number" min={0} max={15} step={0.1} value={p.delay}
                style={{width: 56}} onChange={(e) => set(i, {delay: Number(e.target.value)})} />초</label>}
              <label>사라짐 <select value={p.until} onChange={(e) => set(i, {until: e.target.value as Pick['until']})}
                data-automation-id={`dir-until-${r.id}`}>
                <option value="rule">규칙대로</option><option value="end">장면 끝까지</option><option value="hold">몇 초 동안</option>
              </select></label>
              {p.until === 'hold' && <label><input type="number" min={0.6} max={20} step={0.1} value={p.hold}
                style={{width: 56}} onChange={(e) => set(i, {hold: Number(e.target.value)})} />초</label>}
              <label>크기 <input type="number" min={0.6} max={1.8} step={0.1} value={p.size} style={{width: 56}}
                onChange={(e) => set(i, {size: Number(e.target.value)})} data-automation-id={`dir-size-${r.id}`} />배</label>
              {!r.styled && <>
                <label><input type="checkbox" checked={p.color !== null}
                  onChange={(e) => set(i, {color: e.target.checked ? '#FFFFFF' : null})} /> 색</label>
                {p.color !== null && <input type="color" value={p.color} onChange={(e) => set(i, {color: e.target.value.toUpperCase()})} />}
                <label>굵기 <select value={p.bold === null ? '' : p.bold ? 'b' : 'n'}
                  onChange={(e) => set(i, {bold: e.target.value === '' ? null : e.target.value === 'b'})}>
                  <option value="">그대로</option><option value="b">굵게</option><option value="n">보통</option>
                </select></label>
                <label>글꼴 <select value={p.font ?? ''} onChange={(e) => set(i, {font: (e.target.value || null) as Pick['font']})}>
                  <option value="">그대로</option><option value="base">기본</option><option value="shout">강조체</option>
                  <option value="hand">손글씨</option>
                </select></label>
              </>}
            </div>
            {p.reason && <div className="muted small">{p.manual ? '검수: ' : 'AI: '}{p.reason}</div>}
          </div>
        );
      })}
      <div className="actions">
        <button disabled={!dirty} onClick={() => save(picks)} data-automation-id="dir-save">미리보기에 반영</button>
        <button onClick={allRule} data-automation-id="dir-rule">모두 규칙대로</button>
      </div>
      {msg && <p className="muted small">{msg}</p>}
    </details>
  );
}
