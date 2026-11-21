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
  const busy = st.worker.current !== null;
  const alerts = events.filter((e) => e.type === 'alert' || e.state === 'failed').slice(-5).reverse();
  return (
    <section>
      <h1>홈</h1>
      {/* 지금 할 일 하나만 크게. 나머지 상태는 아래 표로 */}
      {st.review_waiting > 0 ? (
        <div className="next" data-automation-id="home-next">
          <div>
            <p className="lead">검수할 영상 {st.review_waiting}개가 기다립니다</p>
            <p className="muted small">{busy ? `진행 중: ${st.worker.current}` : '지금 만드는 영상은 없습니다'}</p>
          </div>
          <button className="primary" onClick={() => go('검수')} data-automation-id="home-go-review">검수하러 가기</button>
        </div>
      ) : (
        <div className="next" data-automation-id="home-next">
          <div>
            <p className="lead">{busy ? '영상을 만드는 중입니다' : '검수할 영상이 없습니다'}</p>
            <p className="muted small">{busy ? `진행 중: ${st.worker.current} · 끝나면 검수 대기로 옵니다`
              : '갤러리나 게시물 링크를 넣으면 수집부터 미리보기까지 만듭니다'}</p>
          </div>
          <button className="primary" onClick={() => go('새로 만들기')} data-automation-id="home-go-new">링크 넣기</button>
        </div>
      )}
      <table className="facts" aria-label="상태">
        <tbody>
          <tr>
            <th>오늘</th>
            <td className="num">수집 {st.today.collected} · 생성 {st.today.generated} · 승인 {st.today.approved}
              {st.worker.pending > 0 && <span className="muted"> · 대기 {st.worker.pending}</span>}</td>
          </tr>
          <tr>
            <th>크롤링 예산</th>
            <td>
              <Gauge label="이번 시간" used={b.hour_used} limit={b.hour_limit} pct={pct(b.hour_used, b.hour_limit)} />
              <Gauge label="오늘" used={b.day_used} limit={b.day_limit} pct={pct(b.day_used, b.day_limit)} />
              {blocked && <p className="note warn">{b.stopped_today ? '오늘 수집 종료 (두 번 차단됨)' : '차단 뒤 쿨다운 중'}</p>}
              {st.budget_warnings.map((w) => <p key={w} className="note warn">{w}</p>)}
            </td>
          </tr>
          <tr>
            <th>Claude 한도</th>
            <td>
              <span className={st.claude.limited ? 'state-warn' : undefined}>
                {st.claude.limited ? '한도 도달: 로컬 LLM 으로 넘김' : '정상'}</span>
              {st.claude.message && <div className="muted small">{st.claude.message}</div>}
            </td>
          </tr>
          <tr>
            <th>GPU</th>
            <td>{st.gpu.name || '찾지 못함'}
              <div className="muted small">{st.gpu.vendor === 'nvidia' ? '인코딩: NVENC' : '인코딩: CPU (화면 그리기는 GPU)'}</div></td>
          </tr>
        </tbody>
      </table>
      <h3>최근 알림</h3>
      {alerts.length === 0 ? <p className="muted small">새 알림이 없습니다.</p> : (
        <ul className="log">
          {alerts.map((e, i) => (
            <li key={i}><span className="faint">{e.time?.slice(11)}</span>{e.message}</li>
          ))}
        </ul>
      )}
    </section>
  );
}

function Gauge({label, used, limit, pct}: {label: string; used: number; limit: number; pct: number}) {
  return (
    <div className="gauge">
      <div className="gauge-label"><span>{label}</span><span className="num">{used} / {limit}건</span></div>
      <div className="bar" role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
        <div className={pct >= 90 ? 'fill warn' : 'fill'} style={{width: `${pct}%`}} />
      </div>
    </div>
  );
}
