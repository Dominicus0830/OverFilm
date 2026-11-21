import React, {useState} from 'react';
import {api} from '../../api';
import type {Draft} from './types';

export type Turn = {role: 'user' | 'assistant'; text: string};
type Answer = {ok: boolean; error?: string; reply?: string; preset?: Record<string, unknown> | null; summary?: string[]};

/**
 * Claude Code 와 대화로 규칙 정하기 → 요약 확인 → 저장. 학습 결과도 이 대화에 이어져서
 * 대화로 더 고친 뒤 같은 저장 버튼으로 저장한다. 대화·초안 상태는 부모가 가진다.
 */
export function StyleChat({turns, setTurns, draft, setDraft, onSaved}: {
  turns: Turn[]; setTurns: (t: Turn[]) => void; draft: Draft | null; setDraft: (d: Draft | null) => void;
  onSaved: (msg: string) => void;
}) {
  const [text, setText] = useState('');
  const [refs, setRefs] = useState('');
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState('');

  const refList = () => refs.split('\n').map((s) => s.trim()).filter(Boolean).slice(0, 5);
  const send = async () => {
    const t = text.trim();
    if (!t || busy) return;
    const next: Turn[] = [...turns, {role: 'user', text: t}];
    setTurns(next); setText(''); setBusy(true); setMsg('');
    try {
      const r = await api<Answer>('/api/presets/chat',
        {messages: next.slice(-30), references: refList(), current: draft?.preset ?? null});
      if (!r.ok) { setMsg(r.error ?? 'Claude 가 답하지 못했습니다'); return; }
      setTurns([...next, {role: 'assistant', text: r.reply ?? ''}]);
      if (r.preset) setDraft({preset: r.preset, summary: r.summary ?? []});
    } catch (e) { setMsg((e as Error).message); } finally { setBusy(false); }
  };
  const save = async () => {
    if (!draft) return;
    setBusy(true); setMsg('');
    try {
      const r = await api<{id: string}>('/api/presets/save', {preset: draft.preset, references: refList()});
      setTurns([]); setDraft(null); onSaved(`저장했습니다: ${r.id}`);
    } catch (e) { setMsg((e as Error).message); } finally { setBusy(false); }
  };

  return (
    <>
      <p className="muted small">Claude Code 와 대화로 규칙을 정합니다. 저장 버튼을 누르기 전에는 아무것도 저장되지 않고,
        기본 유형은 덮어쓰지 않습니다. Claude 는 영상을 직접 볼 수 없어서 링크·파일 이름과 아래 대화만 참고합니다.</p>
      <label className="field">
        <span>예시 영상 링크·파일 이름 (선택, 한 줄에 하나, 최대 5개)</span>
        <textarea rows={2} value={refs} onChange={(e) => setRefs(e.target.value)} data-automation-id="maker-refs" />
      </label>
      <div className="card">
        {turns.length === 0 && <p className="muted">예: "30초짜리, 댓글은 한 화면에 3개씩, TTS 로 다 읽어줘"</p>}
        {turns.map((t, i) => <p key={i} style={{whiteSpace: 'pre-wrap'}}><b>{t.role === 'user' ? '나' : 'Claude'}</b> {t.text}</p>)}
        <div className="actions">
          <input value={text} onChange={(e) => setText(e.target.value)} disabled={busy}
            placeholder={draft ? '더 고칠 점을 적어 주세요' : '어떤 영상을 만들고 싶은지 적어 주세요'} style={{flex: 1}}
            data-automation-id="maker-input" onKeyDown={(e) => { if (e.key === 'Enter') send(); }} />
          <button className="primary" onClick={send} disabled={busy || !text.trim()} data-automation-id="maker-send">
            {busy ? '기다리는 중…' : '보내기'}</button>
        </div>
      </div>
      {draft && (
        <div className="card" data-automation-id="maker-draft">
          <h3>지금까지 정한 규칙 (검증 통과)</h3>
          <ul>{draft.summary.map((l) => <li key={l}>{l}</li>)}</ul>
          <div className="actions">
            <button className="primary" onClick={save} disabled={busy} data-automation-id="maker-save">저장</button>
            <button onClick={() => setDraft(null)}>버리기</button>
          </div>
        </div>
      )}
      {msg && <p className="error" role="status">{msg}</p>}
    </>
  );
}
