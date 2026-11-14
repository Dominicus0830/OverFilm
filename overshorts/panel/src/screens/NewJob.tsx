import React, {useEffect, useState} from 'react';
import {api} from '../api';
import type {Screen} from '../App';

type Parsed = {ok: boolean; kind?: string; label?: string; error?: string; recommend_only?: boolean};

export function NewJob({go}: {go: (s: Screen) => void}) {
  const [url, setUrl] = useState('');
  const [parsed, setParsed] = useState<Parsed | null>(null);
  const [advanced, setAdvanced] = useState(false);
  const [recommendOnly, setRecommendOnly] = useState(true);
  const [minUp, setMinUp] = useState(20);
  const [hours, setHours] = useState(24);
  const [maxPosts, setMaxPosts] = useState(5);
  const [tone, setTone] = useState('담백');
  const [msg, setMsg] = useState('');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!url.trim()) { setParsed(null); return; }
    const t = setTimeout(() => {
      api<Parsed>('/api/parse', {url}).then(setParsed).catch((e) => setParsed({ok: false, error: e.message}));
    }, 250);
    return () => clearTimeout(t);
  }, [url]);

  const gallery = parsed?.ok && parsed.kind === 'gallery';
  const start = async () => {
    setBusy(true); setMsg('');
    try {
      await api('/api/start', {url, recommend_only: recommendOnly, min_upvotes: minUp,
        within_hours: hours, max_posts: maxPosts, tone});
      go('작업 큐');
    } catch (e) {
      setMsg((e as Error).message);
    } finally { setBusy(false); }
  };

  return (
    <section>
      <h1>새로 만들기</h1>
      <label className="field">
        <span>DC 갤러리 링크나 게시물 링크</span>
        <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://gall.dcinside.com/mgallery/board/lists/?id=…"
          data-automation-id="new-url" autoFocus />
      </label>
      {parsed && (parsed.ok
        ? <p className="badge ok">{parsed.label}</p>
        : <p className="badge warn">알아볼 수 없는 링크: {parsed.error}</p>)}
      {gallery && (
        <p className="badge warn">갤러리 모드는 목록을 훑어 여러 글을 받습니다. 요청 예산 안에서만 돌고, 차단 신호가 오면 바로 멈춥니다.</p>
      )}
      <label className="field">
        <span>톤</span>
        <select value={tone} onChange={(e) => setTone(e.target.value)}>
          <option>담백</option><option>드립 조금</option><option>정리형</option>
        </select>
      </label>
      <button className="link" onClick={() => setAdvanced(!advanced)}>{advanced ? '고급 접기' : '고급'}</button>
      {advanced && gallery && (
        <div className="card">
          <label className="check"><input type="checkbox" checked={recommendOnly} onChange={(e) => setRecommendOnly(e.target.checked)} /> 개념글만</label>
          <Slider label="최소 추천 수" value={minUp} min={0} max={200} step={5} set={setMinUp} hint="기본 20" />
          <Slider label="최근 몇 시간" value={hours} min={1} max={168} step={1} set={setHours} hint="기본 24" />
          <Slider label="최대 글 수" value={maxPosts} min={1} max={20} step={1} set={setMaxPosts} hint="기본 5. 많을수록 요청이 늘어 차단 위험" />
        </div>
      )}
      <div className="actions">
        <button className="primary" disabled={!parsed?.ok || busy} onClick={start} data-automation-id="new-start">시작</button>
      </div>
      {msg && <p className="error">{msg}</p>}
    </section>
  );
}

function Slider({label, value, min, max, step, set, hint}: {label: string; value: number; min: number; max: number;
  step: number; set: (v: number) => void; hint: string}) {
  return (
    <label className="field">
      <span>{label}: <b>{value}</b> <span className="muted small">{hint}</span></span>
      <input type="range" min={min} max={max} step={step} value={value} onChange={(e) => set(Number(e.target.value))} />
    </label>
  );
}
