import React, {useState} from 'react';
import {api, apiDelete} from '../../api';
import {Dialog} from '../../ui/Dialog';
import type {PresetInfo} from './types';

const NOWRAP: React.CSSProperties = {whiteSpace: 'nowrap'};

/**
 * 스타일 목록: 복사(새 이름으로 복제해 바로 저장)·학습으로 보충·지우기(내 유형만).
 * 목록을 다시 읽는 것은 부모가 한다(`onChanged`).
 */
export function StyleList({list, onChanged, onSupplement, onMessage}: {
  list: PresetInfo[]; onChanged: () => void; onSupplement: (id: string) => void; onMessage: (m: string) => void;
}) {
  const [copying, setCopying] = useState<PresetInfo | null>(null);
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);

  const openCopy = (p: PresetInfo) => { setCopying(p); setName(`${p.label} 복사`.slice(0, 20)); };
  const copy = async () => {
    if (!copying) return;
    setBusy(true);
    try {
      const r = await api<{id: string; label: string}>(`/api/presets/${encodeURIComponent(copying.id)}/copy`,
        {label: name.trim()});
      onMessage(`복사했습니다: ${r.label} (${r.id})`);
      setCopying(null); onChanged();
    } catch (e) { onMessage((e as Error).message); } finally { setBusy(false); }
  };
  const remove = async (id: string) => {
    try { await apiDelete(`/api/presets/${encodeURIComponent(id)}`); onChanged(); }
    catch (e) { onMessage((e as Error).message); }
  };

  return (
    <>
      <table className="jobs" aria-label="편집 유형">
        <thead><tr><th>유형</th><th>종류</th><th>규칙</th><th /></tr></thead>
        <tbody>
          {list.map((p) => (
            <tr key={p.id}>
              <td style={NOWRAP}><b>{p.label}</b></td>
              <td className="muted" style={NOWRAP}>{p.origin === 'custom' ? '내 유형' : '기본'}</td>
              <td className="muted small">{p.summary.slice(1).join(' · ')}</td>
              <td style={NOWRAP}>
                <button className="small-btn" onClick={() => openCopy(p)} data-automation-id={`style-copy-${p.id}`}>
                  복사</button>{' '}
                <button className="small-btn" onClick={() => onSupplement(p.id)}
                  title="학습 자료로 이 스타일을 보충" data-automation-id={`style-supplement-${p.id}`}>보충</button>{' '}
                {p.origin === 'custom' && <button className="small-btn danger" onClick={() => remove(p.id)}
                  data-automation-id={`style-remove-${p.id}`}>지우기</button>}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <Dialog open={copying !== null} title="스타일 복사" onDismiss={() => setCopying(null)}
        automationId="style-copy-dialog">
        {copying && (<>
          <p className="muted">'{copying.label}'의 규칙을 그대로 복제해 내 유형으로 저장합니다. 원본은 바뀌지 않습니다.</p>
          <label className="field">
            <span>새 이름 (20자 이내)</span>
            <input value={name} maxLength={20} onChange={(e) => setName(e.target.value)} autoFocus
              data-automation-id="style-copy-name" onKeyDown={(e) => { if (e.key === 'Enter') copy(); }} />
          </label>
          <div className="actions">
            <button onClick={() => setCopying(null)}>취소</button>
            <button className="primary" onClick={copy} disabled={busy} data-automation-id="style-copy-confirm">
              복사</button>
          </div>
        </>)}
      </Dialog>
    </>
  );
}
