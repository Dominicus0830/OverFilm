import React, {useEffect, useState} from 'react';
import {api} from '../api';
import type {PresetInfo} from './EditOptions';

type Turn = {role: 'user' | 'assistant'; text: string};
type Answer = {ok: boolean; error?: string; reply?: string; preset?: Record<string, unknown> | null; summary?: string[]};

/** 커스텀 유형 만들기: Claude Code 와 대화 → 요약 확인 → 저장 버튼을 눌러야 저장. */
export function StyleMaker() {
  const [list, setList] = useState<PresetInfo[]>([]);
  const [turns, setTurns] = useState<Turn[]>([]);
  const [text, setText] = useState('');
  const [refs, setRefs] = useState('');
  const [preset, setPreset] = useState<Record<string, unknown> | null>(null);
  const [summary, setSummary] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState('');

  const load = () => api<PresetInfo[]>('/api/presets').then(setList).catch((e) => setMsg(e.message));
  useEffect(() => { load(); }, []);

  const refList = () => refs.split('\n').map((s) => s.trim()).filter(Boolean).slice(0, 5);
  const send = async () => {
    const t = text.trim();
    if (!t || busy) return;
    const next: Turn[] = [...turns, {role: 'user', text: t}];
    setTurns(next); setText(''); setBusy(true); setMsg('');
    try {
      const r = await api<Answer>('/api/presets/chat', {messages: next, references: refList(), current: preset});
      if (!r.ok) { setMsg(r.error ?? 'Claude 가 답하지 못했습니다'); return; }
      setTurns([...next, {role: 'assistant', text: r.reply ?? ''}]);
      if (r.preset) { setPreset(r.preset); setSummary(r.summary ?? []); }
    } catch (e) { setMsg((e as Error).message); } finally { setBusy(false); }
  };
  const save = async () => {
    if (!preset) return;
    setBusy(true); setMsg('');
    try {
      const r = await api<{id: string}>('/api/presets/save', {preset, references: refList()});
      setMsg(`저장했습니다: ${r.id}`);
      setTurns([]); setPreset(null); setSummary([]); load();
    } catch (e) { setMsg((e as Error).message); } finally { setBusy(false); }
  };
  const remove = async (id: string) => {
    try { await fetch(`/api/presets/${encodeURIComponent(id)}`, {method: 'DELETE'}).then((r) => {
      if (!r.ok) throw new Error('지우지 못했습니다');
    }); load(); } catch (e) { setMsg((e as Error).message); }
  };

  return (
    <section>
      <h1>스타일 (편집 유형)</h1>
      <table className="jobs" aria-label="편집 유형">
        <thead><tr><th>유형</th><th>종류</th><th>규칙</th><th /></tr></thead>
        <tbody>
          {list.map((p) => (
            <tr key={p.id}>
              <td><b>{p.label}</b></td>
              <td className="muted">{p.origin === 'custom' ? '내 유형' : '기본'}</td>
              <td className="muted small">{p.summary.slice(1).join(' · ')}</td>
              <td>{p.origin === 'custom' && <button className="small-btn" onClick={() => remove(p.id)}
                data-automation-id={`style-remove-${p.id}`}>지우기</button>}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <h2>내 유형 만들기</h2>
      <p className="muted small">Claude Code 와 대화로 규칙을 정합니다. 저장 버튼을 누르기 전에는 아무것도 저장되지 않고,
        기본 유형은 덮어쓰지 않습니다. Claude 는 영상을 직접 볼 수 없어서 링크·파일 이름과 아래 대화만 참고합니다.</p>
      <label className="field">
        <span>예시 영상 링크·파일 이름 (선택, 한 줄에 하나, 최대 5개)</span>
        <textarea rows={2} value={refs} onChange={(e) => setRefs(e.target.value)} data-automation-id="maker-refs" />
      </label>
      <div className="card">
        {turns.length === 0 && <p className="muted">예: "30초짜리, 댓글은 한 화면에 3개씩, TTS 로 다 읽어줘"</p>}
        {turns.map((t, i) => <p key={i}><b>{t.role === 'user' ? '나' : 'Claude'}</b> {t.text}</p>)}
        <div className="actions">
          <input value={text} onChange={(e) => setText(e.target.value)} disabled={busy}
            placeholder="어떤 영상을 만들고 싶은지 적어 주세요" style={{flex: 1}} data-automation-id="maker-input"
            onKeyDown={(e) => { if (e.key === 'Enter') send(); }} />
          <button className="primary" onClick={send} disabled={busy || !text.trim()} data-automation-id="maker-send">
            {busy ? '기다리는 중…' : '보내기'}</button>
        </div>
      </div>
      {preset && (
        <div className="card">
          <h3>지금까지 정한 규칙 (검증 통과)</h3>
          <ul>{summary.map((l) => <li key={l}>{l}</li>)}</ul>
          <div className="actions">
            <button className="primary" onClick={save} disabled={busy} data-automation-id="maker-save">저장</button>
            <button onClick={() => { setPreset(null); setSummary([]); }}>버리기</button>
          </div>
        </div>
      )}
      {msg && <p className={msg.startsWith('저장') ? 'note ok' : 'error'} role="status">{msg}</p>}
    </section>
  );
}
