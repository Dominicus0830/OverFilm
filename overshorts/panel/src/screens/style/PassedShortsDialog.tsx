import React, {useEffect, useState} from 'react';
import {api} from '../../api';
import {Dialog} from '../../ui/Dialog';
import type {PassedShort} from './types';

const MAX = 10;

/** 검수를 통과한 쇼츠 중에서 학습에 쓸 것을 고르는 작은 창 (최대 10개). */
export function PassedShortsDialog({open, picked, onClose, onPick}: {
  open: boolean; picked: number[]; onClose: () => void; onPick: (ids: number[]) => void;
}) {
  const [list, setList] = useState<PassedShort[] | null>(null);
  const [sel, setSel] = useState<number[]>(picked);
  const [err, setErr] = useState('');

  useEffect(() => {
    if (!open) return;
    setSel(picked); setErr('');
    api<PassedShort[]>('/api/presets/passed').then(setList).catch((e) => setErr((e as Error).message));
  }, [open]); // eslint-disable-line react-hooks/exhaustive-deps

  const toggle = (id: number) => setSel((s) => s.includes(id) ? s.filter((x) => x !== id)
    : s.length < MAX ? [...s, id] : s);

  return (
    <Dialog open={open} title="통과된 쇼츠에서 고르기" onDismiss={onClose} automationId="style-passed-dialog">
      {err && <p className="error">{err}</p>}
      {list === null && !err && <p className="muted">불러오는 중…</p>}
      {list !== null && list.length === 0 && <p className="muted">아직 검수를 통과한 쇼츠가 없습니다.</p>}
      {list !== null && list.length > 0 && (
        <div style={{maxHeight: 320, overflowY: 'auto'}}>
          {list.map((s) => (
            <label key={s.id} className="row" style={{display: 'flex', gap: 8, alignItems: 'center', padding: '2px 0'}}>
              <input type="checkbox" checked={sel.includes(s.id)} onChange={() => toggle(s.id)}
                data-automation-id={`style-passed-${s.id}`} />
              <span>{s.title || `작업 ${s.id}`}</span>
              <span className="muted small">{s.status === 'rendered' ? '완성' : '통과'} · {s.updated_at.slice(0, 10)}</span>
            </label>
          ))}
        </div>
      )}
      <p className="muted small">{sel.length}개 고름 (최대 {MAX}개)</p>
      <div className="actions">
        <button onClick={onClose}>취소</button>
        <button className="primary" onClick={() => { onPick(sel); onClose(); }}
          data-automation-id="style-passed-confirm">고르기</button>
      </div>
    </Dialog>
  );
}
