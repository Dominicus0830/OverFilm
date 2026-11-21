import React, {useEffect, useState} from 'react';
import {api, apiDelete, Event, Job, WorkerState} from '../api';
import {Dialog} from '../ui/Dialog';

const STEPS = ['crawled', 'filtered', 'scripted', 'aligned', 'preview_ready', 'approved', 'rendered'];
const KO: Record<string, string> = {
  queued: '대기', crawled: '수집됨', filtered: '필터 통과', scripted: '대본', voiced: '음성', aligned: '자막 타이밍',
  preview_ready: '검수 대기', approved: '승인 (렌더 중)', rendered: '완료', blocked: '막힘', rejected: '폐기', failed: '실패',
};

function statusText(j: Job): string {
  if (j.status === 'approved' && j.error) return '렌더 실패';
  return KO[j.status] ?? j.status;
}

export function Queue({tick, events}: {tick: number; events: Event[]}) {
  const [jobs, setJobs] = useState<Job[]>([]);
  const [worker, setWorker] = useState<WorkerState | null>(null);
  const [err, setErr] = useState('');
  const [toDelete, setToDelete] = useState<Job | null>(null);
  const [busy, setBusy] = useState(false);
  const load = () => {
    api<Job[]>('/api/jobs').then(setJobs).catch((e) => setErr(e.message));
    api<WorkerState>('/api/worker').then(setWorker).catch((e) => setErr(e.message));
  };
  const last = events[events.length - 1];
  useEffect(load, [tick, last?.type === 'queue' || last?.type === 'task' ? events.length : 0]);
  const progress = events.filter((e) => e.type === 'progress' || e.type === 'task').slice(-1)[0];
  const paused = worker?.paused ?? false;
  const act = async (fn: () => Promise<unknown>) => {
    setErr('');
    setBusy(true);
    try { await fn(); } catch (e) { setErr((e as Error).message); } finally { setBusy(false); load(); }
  };
  const togglePause = () => act(() => api(`/api/worker/pause?paused=${!paused}`, {}));
  const cancelCurrent = () => act(() => api(`/api/worker/cancel${worker?.current ? `?task_id=${worker.current.id}` : ''}`, {}));
  const removeTask = (id: number) => act(() => apiDelete(`/api/worker/tasks/${id}`));
  const deleteJob = () => {
    const j = toDelete;
    setToDelete(null);
    if (j) act(() => apiDelete(`/api/jobs/${j.id}`));
  };
  return (
    <section>
      <h1>작업 큐</h1>
      <div className="actions">
        <span className="muted">{progress ? `${progress.label ?? progress.stage ?? ''} ${progress.message ?? progress.state ?? ''}` : '쉬는 중'}</span>
        <button onClick={togglePause} data-automation-id="queue-pause">{paused ? '다시 시작' : '일시정지'}</button>
      </div>
      {err && <p className="error">{err}</p>}
      {worker && (worker.current || worker.pending.length > 0) && (
        <ul className="tasks" data-automation-id="queue-tasks">
          {worker.current && (
            <li>
              <span className="state-warn">{worker.cancelling ? '멈추는 중' : '실행 중'}</span>
              <span className="grow">{worker.current.label}</span>
              <button className="small-btn danger" disabled={busy || worker.cancelling} onClick={cancelCurrent}
                data-automation-id="queue-cancel-current">취소</button>
            </li>
          )}
          {worker.pending.map((t) => (
            <li key={t.id}>
              <span className="muted">대기</span>
              <span className="grow">{t.label}</span>
              <button className="small-btn" disabled={busy} onClick={() => removeTask(t.id)}
                data-automation-id={`queue-remove-${t.id}`}>빼기</button>
            </li>
          ))}
        </ul>
      )}
      {jobs.length === 0 && <p className="muted">아직 작업이 없습니다. ‘새로 만들기’에서 링크를 넣으세요.</p>}
      {jobs.length > 0 && (<table className="jobs">
        <thead><tr><th>#</th><th>글</th><th>진행</th><th>상태</th><th /></tr></thead>
        <tbody>
          {jobs.map((j) => {
            const idx = STEPS.indexOf(j.status);
            const pct = idx < 0 ? 0 : Math.round(((idx + 1) / STEPS.length) * 100);
            return (
              <tr key={j.id}>
                <td className="num muted">{j.id}</td>
                <td className="title-cell">{j.title || `${j.gall_id} ${j.post_no}`}</td>
                <td><div className="bar"><div className={j.error ? 'fill warn' : 'fill'} style={{width: `${pct}%`}} /></div></td>
                <td>
                  <span className={j.status === 'blocked' || j.error ? 'state-warn' : j.status === 'rendered' ? 'state-ok' : undefined}>
                    {statusText(j)}</span>
                  {j.error && <div className="muted small">{j.error}</div>}
                </td>
                <td><div className="row-actions">
                  {j.error && j.status !== 'rejected' && (
                    <button className="small-btn" disabled={busy} onClick={() => act(() => api(`/api/jobs/${j.id}/retry`, {}))}>이 단계부터 다시</button>)}
                  <button className="small-btn danger" disabled={busy} onClick={() => setToDelete(j)}
                    data-automation-id={`job-delete-${j.id}`}>삭제</button>
                </div></td>
              </tr>
            );
          })}
        </tbody>
      </table>)}
      <Dialog open={toDelete !== null} title="이 작업을 지울까요?" onDismiss={() => setToDelete(null)}
        automationId="job-delete-dialog">
        {toDelete && (<>
          <p><b>{toDelete.title || `${toDelete.gall_id} ${toDelete.post_no}`}</b></p>
          <p className="muted">작업 큐에서 빠지고 중간 파일(음성·미리보기)도 지웁니다. 다 만든 영상 파일은 남습니다.
            지금 이 작업을 처리하는 중이면 멈춥니다. 같은 글은 다시 넣으면 새로 만들 수 있습니다.</p>
          <div className="actions">
            <button onClick={() => setToDelete(null)}>그대로 두기</button>
            <button className="primary" onClick={deleteJob} data-automation-id="job-delete-confirm">삭제</button>
          </div>
        </>)}
      </Dialog>
      <h3>로그</h3>
      <ul className="log">
        {events.slice(-30).reverse().map((e, i) => (
          <li key={i}><span className="faint">{e.time?.slice(11)}</span>{e.label ?? e.stage ?? e.type} {e.message ?? e.state ?? ''}</li>
        ))}
      </ul>
    </section>
  );
}
