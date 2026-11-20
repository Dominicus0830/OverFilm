import React, {useEffect, useState} from 'react';
import {api} from '../api';
import type {Screen} from '../App';

export type PresetInfo = {id: string; label: string; origin: string; summary: string[]};
type SettingsView = {values: Record<string, Record<string, unknown>>};

const on = (v: unknown) => ['true', 'on', 'yes', '1', '켬'].includes(String(v ?? '').toLowerCase());

/** 편집 스타일 유형 + 순화·린터·전사 대조 토글. 값은 설정(config.yaml)에 저장돼 다음 작업부터 쓰인다. */
export function EditOptions({go, onReady}: {go: (s: Screen) => void; onReady: (apply: () => Promise<void>) => void}) {
  const [presets, setPresets] = useState<PresetInfo[]>([]);
  const [type, setType] = useState('ogal');
  const [soften, setSoften] = useState(false);
  const [lint, setLint] = useState(false);
  const [verify, setVerify] = useState(false);
  const [err, setErr] = useState('');

  useEffect(() => {
    api<PresetInfo[]>('/api/presets').then(setPresets).catch((e) => setErr(e.message));
    api<SettingsView>('/api/settings').then((s) => {
      setType(String(s.values.render?.edit_type || 'ogal'));
      setSoften(on(s.values.script?.soften)); setLint(on(s.values.script?.lint));
      setVerify(on(s.values.verify?.enabled));
    }).catch(() => undefined);
  }, []);

  // 시작 버튼이 누르기 직전에 호출: 고른 값을 설정에 쓴다
  useEffect(() => {
    onReady(async () => {
      await api('/api/settings', {values: {render: {edit_type: type},
        script: {soften: soften ? 'on' : 'off', lint: lint ? 'on' : 'off'}, verify: {enabled: verify ? 'on' : 'off'}}});
    });
  }, [type, soften, lint, verify, onReady]);

  const cur = presets.find((p) => p.id === type);
  const order = ['issue', 'ogal', 'info', 'game'];
  const sorted = [...presets].sort((a, b) => {
    const ia = order.indexOf(a.id), ib = order.indexOf(b.id);
    return (ia < 0 ? 9 : ia) - (ib < 0 ? 9 : ib);
  });
  return (
    <div className="card">
      <label className="field">
        <span>편집 스타일 유형</span>
        <select value={type} onChange={(e) => setType(e.target.value)} data-automation-id="new-edit-type">
          {sorted.map((p) => <option key={p.id} value={p.id}>{p.label}{p.origin === 'custom' ? ' (내 유형)' : ''}</option>)}
          {!cur && <option value={type}>{type}</option>}
        </select>
      </label>
      {cur && <ul className="muted small">{cur.summary.slice(1, 5).map((l) => <li key={l}>{l}</li>)}</ul>}
      <button className="link" onClick={() => go('스타일')} data-automation-id="new-make-type">내 유형 만들기</button>
      <p className="muted small">옵션은 기본 꺼짐 = 원문 내용 그대로.</p>
      <label className="check"><input type="checkbox" checked={soften} onChange={(e) => setSoften(e.target.checked)}
        data-automation-id="opt-soften" /> 순화 (욕설·혐오 표현을 피해서 씀)</label>
      <label className="check"><input type="checkbox" checked={lint} onChange={(e) => setLint(e.target.checked)}
        data-automation-id="opt-lint" /> 대본 린터 (개인정보·금지 표현·길이 검사, 내용은 안 고침)</label>
      <label className="check"><input type="checkbox" checked={verify} onChange={(e) => setVerify(e.target.checked)}
        data-automation-id="opt-verify" /> 전사 대조 (만든 음성을 다시 받아 대본과 비교)</label>
      {err && <p className="error">{err}</p>}
    </div>
  );
}
