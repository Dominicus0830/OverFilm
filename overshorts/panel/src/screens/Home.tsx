import React, {useEffect, useState} from 'react';
import {api, Event, Status} from '../api';
import type {Screen} from '../App';

export function Home({tick, go, events}: {tick: number; go: (s: Screen) => void; events: Event[]}) {
  const [st, setSt] = useState<Status | null>(null);
  const [err, setErr] = useState('');
  useEffect(() => {
    api<Status>('/api/status').then(setSt).catch((e) => setErr(String(e.message ?? e)));
  }, [tick]);
  if (err) return <p className="error">상태를 읽지 못했습니다: {err}</p>;
  if (!st) return <p className="muted">불러오는 중…</p>;
  const b = st.budget;
  const pct = (u: number, l: number) => (l > 0 ? Math.min(100, Math.round((u / l) * 100)) : 100);
  const blocked = b.stopped_today || b.cooldown_until !== null;
  return (
    <section>
      <h1>홈</h1>
      <div className="cards">
        <div className="card stat"><span>오늘 수집</span><b>{st.today.collected}</b></div>
        <div className="card stat"><span>오늘 생성</span><b>{st.today.generated}</b></div>
        <div className="card stat"><span>오늘 승인</span><b>{st.today.approved}</b></div>
      </div>
      <div className="cards">
        <div className="card">
          <h3>크롤링 예산</h3>
          <Gauge label="이번 시간" used={b.hour_used} limit={b.hour_limit} pct={pct(b.hour_used, b.hour_limit)} />
          <Gauge label="오늘" used={b.day_used} limit={b.day_limit} pct={pct(b.day_used, b.day_limit)} />
          {blocked && <p className="badge warn">{b.stopped_today ? '오늘 수집 종료 (두 번 차단됨)' : '차단 뒤 쿨다운 중'}</p>}
          {st.budget_warnings.map((w) => <p key={w} className="badge warn">{w}</p>)}
        </div>
        <div className="card">
          <h3>Claude 한도</h3>
          <p className={st.claude.limited ? 'badge warn' : 'badge ok'}>
            {st.claude.limited ? '한도 도달: 로컬 LLM 으로 넘김' : '정상'}</p>
          {st.claude.message && <p className="muted small">{st.claude.message}</p>}
        </div>
        <div className="card">
          <h3>GPU</h3>
          <p>{st.gpu.name || '찾지 못함'}</p>
          <p className="muted small">{st.gpu.vendor === 'nvidia' ? '인코딩: NVENC' : '인코딩: CPU (화면 그리기는 GPU)'}</p>
        </div>
      </div>
      {st.review_waiting === 0 && st.worker.current === null ? (
        <div className="card next">
          <p>검수할 영상이 없습니다.</p>
          <button className="primary" onClick={() => go('새로 만들기')}>링크 넣기</button>
        </div>
      ) : (
        <div className="card next">
          <p>검수 대기 {st.review_waiting}개{st.worker.current ? ` · 진행 중: ${st.worker.current}` : ''}</p>
          <button className="primary" onClick={() => go('검수')}>검수하러 가기</button>
        </div>
      )}
      <h3>최근 알림</h3>
      <ul className="log">
        {events.filter((e) => e.type === 'alert' || e.state === 'failed').slice(-5).reverse().map((e, i) => (
          <li key={i}><span className="muted">{e.time?.slice(11)}</span> {e.message}</li>
        ))}
      </ul>
    </section>
  );
}

function Gauge({label, used, limit, pct}: {label: string; used: number; limit: number; pct: number}) {
  return (
    <div className="gauge">
      <div className="gauge-label"><span>{label}</span><span>{used} / {limit}건</span></div>
      <div className="bar" role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
        <div className={pct >= 90 ? 'fill warn' : 'fill'} style={{width: `${pct}%`}} />
      </div>
    </div>
  );
}
