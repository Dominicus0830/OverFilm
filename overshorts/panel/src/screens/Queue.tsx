import React, {useEffect, useState} from 'react';
import {api, Event, Job} from '../api';

const STEPS = ['crawled', 'filtered', 'scripted', 'aligned', 'preview_ready', 'approved', 'rendered'];
const KO: Record<string, string> = {
  queued: '대기', crawled: '수집됨', filtered: '필터 통과', scripted: '대본', voiced: '음성', aligned: '자막 타이밍',
  preview_ready: '검수 대기', approved: '승인 (렌더 중)', rendered: '완료', blocked: '막힘', rejected: '폐기', failed: '실패',
};

export function Queue({tick, events}: {tick: number; events: Event[]}) {
  const [jobs, setJobs] = useState<Job[]>([]);
  const [paused, setPaused] = useState(false);
  const [err, setErr] = useState('');
  useEffect(() => { api<Job[]>('/api/jobs').then(setJobs).catch((e) => setErr(e.message)); }, [tick]);
  const progress = events.filter((e) => e.type === 'progress' || e.type === 'task').slice(-1)[0];
  const togglePause = async () => {
    const r = await api<{paused: boolean}>(`/api/worker/pause?paused=${!paused}`, {});
    setPaused(r.paused);
  };
  return (
    <section>
      <h1>작업 큐</h1>
      <div className="actions">
        <span className="muted">{progress ? `${progress.label ?? progress.stage ?? ''} ${progress.message ?? progress.state ?? ''}` : '쉬는 중'}</span>
        <button onClick={togglePause}>{paused ? '다시 시작' : '일시정지'}</button>
      </div>
      {err && <p className="error">{err}</p>}
      {jobs.length === 0 && <p className="muted">아직 작업이 없습니다. ‘새로 만들기’에서 링크를 넣으세요.</p>}
      <table className="jobs">
        <thead><tr><th>#</th><th>글</th><th>진행</th><th>상태</th><th /></tr></thead>
        <tbody>
          {jobs.map((j) => {
            const idx = STEPS.indexOf(j.status);
            const pct = idx < 0 ? 0 : Math.round(((idx + 1) / STEPS.length) * 100);
            return (
              <tr key={j.id}>
                <td>{j.id}</td>
                <td className="title-cell">{j.title || `${j.gall_id} ${j.post_no}`}</td>
                <td><div className="bar"><div className={j.error ? 'fill warn' : 'fill'} style={{width: `${pct}%`}} /></div></td>
                <td>
                  <span className={j.status === 'blocked' || j.error ? 'badge warn' : 'badge'}>{KO[j.status] ?? j.status}</span>
                  {j.error && <div className="muted small">{j.error}</div>}
                </td>
                <td>{j.error && j.status !== 'rejected' && (
                  <button onClick={() => api(`/api/jobs/${j.id}/retry`, {})}>이 단계부터 다시</button>)}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <h3>로그</h3>
      <ul className="log">
        {events.slice(-30).reverse().map((e, i) => (
          <li key={i}><span className="muted">{e.time?.slice(11)}</span> {e.label ?? e.stage ?? e.type} {e.message ?? e.state ?? ''}</li>
        ))}
      </ul>
    </section>
  );
}
