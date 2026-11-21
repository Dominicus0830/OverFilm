import React, {useState} from 'react';
import {api} from '../../api';
import {PassedShortsDialog} from './PassedShortsDialog';
import type {LearnAnswer, PresetInfo} from './types';

const MAX_FILE = 64 * 1024 * 1024;

function readBase64(file: File): Promise<string> {
  return new Promise((ok, fail) => {
    const r = new FileReader();
    r.onload = () => ok(String(r.result ?? ''));
    r.onerror = () => fail(new Error('파일을 읽지 못했습니다'));
    r.readAsDataURL(file);
  });
}

/**
 * 스타일 학습: Premiere 파일·통과된 쇼츠·유튜브 링크 중 하나 이상으로 Claude Code 가 특징을 모아
 * 기존 유형을 보충하거나 새 유형을 만든다. 결과는 저장 전 미리 보기로 `onResult` 에 넘긴다.
 */
export function StyleLearn({list, base, setBase, onResult}: {
  list: PresetInfo[]; base: string; setBase: (id: string) => void; onResult: (a: LearnAnswer) => void;
}) {
  const [file, setFile] = useState<File | null>(null);
  const [jobs, setJobs] = useState<number[]>([]);
  const [links, setLinks] = useState('');
  const [note, setNote] = useState('');
  const [picking, setPicking] = useState(false);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState('');

  const linkList = () => links.split('\n').map((s) => s.trim()).filter(Boolean).slice(0, 5);
  const ready = !!file || jobs.length > 0 || linkList().length > 0;

  const run = async () => {
    if (!ready || busy) return;
    if (file && file.size > MAX_FILE) { setErr('Premiere 파일이 너무 큽니다 (64MB 까지)'); return; }
    setBusy(true); setErr('');
    try {
      const premiere = file ? {name: file.name, data: await readBase64(file)} : null;
      const r = await api<LearnAnswer>('/api/presets/learn',
        {base: base || null, premiere, jobs, links: linkList(), note});
      if (!r.ok) { setErr(r.error ?? 'Claude 가 답하지 못했습니다'); return; }
      onResult(r);
    } catch (e) { setErr((e as Error).message); } finally { setBusy(false); }
  };

  return (
    <div className="card" data-automation-id="style-learn">
      <h3>학습으로 스타일 만들기·보충</h3>
      <p className="muted small">자료에서 특징(컷 템포·구성·댓글 비중·TTS·자막)을 Claude Code 가 모아 규칙으로 만듭니다.
        무엇을 넣을지는 Claude 가 판단하고, 저장 버튼을 누르기 전에는 저장되지 않습니다.</p>
      <label className="field">
        <span>대상</span>
        <select value={base} onChange={(e) => setBase(e.target.value)} data-automation-id="style-learn-base">
          <option value="">새 스타일 만들기</option>
          {list.map((p) => <option key={p.id} value={p.id}>보충: {p.label}{p.origin === 'custom' ? '' : ' (기본 → 내 유형으로 저장)'}</option>)}
        </select>
      </label>
      <label className="field">
        <span>Premiere Pro 파일 (.prproj, 또는 Premiere 에서 내보낸 .xml)</span>
        <input type="file" accept=".prproj,.xml" onChange={(e) => setFile(e.target.files?.[0] ?? null)}
          data-automation-id="style-learn-premiere" />
      </label>
      <div className="field">
        <span>통과된 쇼츠 (내가 만든 것 중에서)</span>
        <div className="actions" style={{margin: 0}}>
          <button onClick={() => setPicking(true)} data-automation-id="style-learn-pick">목록에서 고르기…</button>
          <span className="muted small">{jobs.length ? `${jobs.length}개 고름` : '고르지 않음'}</span>
          {jobs.length > 0 && <button className="small-btn" onClick={() => setJobs([])}>비우기</button>}
        </div>
      </div>
      <label className="field">
        <span>유튜브 링크 (한 줄에 하나, 최대 5개 · Claude Code 가 웹에서 특징을 모읍니다)</span>
        <textarea rows={2} value={links} onChange={(e) => setLinks(e.target.value)} data-automation-id="style-learn-links" />
      </label>
      <label className="field">
        <span>메모 (선택: 특히 따라 하고 싶은 점)</span>
        <input value={note} maxLength={1000} onChange={(e) => setNote(e.target.value)} data-automation-id="style-learn-note" />
      </label>
      <div className="actions">
        <button className="primary" onClick={run} disabled={!ready || busy} data-automation-id="style-learn-run">
          {busy ? '학습하는 중…' : base ? '학습해서 보충' : '학습해서 새로 만들기'}</button>
      </div>
      {err && <p className="error" role="status">{err}</p>}
      <PassedShortsDialog open={picking} picked={jobs} onClose={() => setPicking(false)} onPick={setJobs} />
    </div>
  );
}
