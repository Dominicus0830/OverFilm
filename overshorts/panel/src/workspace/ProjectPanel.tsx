import React, {useState} from 'react';
import {api, apiDelete, Job, WorkerState} from '../api';
import {Dialog} from '../ui/Dialog';
import {Icon} from '../ui/Icon';
import {GROUPS, Group, groupOf, jobName, statusText} from './jobs';

const TONE: Record<Group, string> = {review: 'state-ok', run: 'state-warn', wait: 'muted', fail: 'error', done: 'muted', rejected: 'faint'};

/**
 * 왼쪽 '프로젝트' 패널: 새로 만들기 + 글 목록(예전 작업 큐) + 아래 작업 대기열(취소·빼기·일시정지).
 * 글을 고르면 가운데 모니터·오른쪽 속성·아래 타임라인이 그 글로 바뀐다.
 */
export function ProjectPanel({jobs, worker, selected, onSelect, onNew, onShowLog, reload}: {
  jobs: Job[]; worker: WorkerState | null; selected: number | null; onSelect: (id: number) => void;
  onNew: () => void; onShowLog: () => void; reload: () => void;
}) {
  const [q, setQ] = useState('');
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  const [toDelete, setToDelete] = useState<Job | null>(null);
  const [showRejected, setShowRejected] = useState(false);

  const act = async (fn: () => Promise<unknown>) => {
    setErr(''); setBusy(true);
    try { await fn(); } catch (e) { setErr((e as Error).message); } finally { setBusy(false); reload(); }
  };
  const paused = worker?.paused ?? false;
  const shown = jobs.filter((j) => !q.trim() || jobName(j).includes(q.trim()) || String(j.post_no).includes(q.trim()));

  const onKey = (e: React.KeyboardEvent, list: Job[]) => {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    e.preventDefault();
    const i = list.findIndex((j) => j.id === selected);
    const next = list[Math.min(list.length - 1, Math.max(0, i + (e.key === 'ArrowDown' ? 1 : -1)))];
    if (next) {
      onSelect(next.id);
      document.querySelector<HTMLElement>(`[data-automation-id="job-${next.id}"]`)?.focus();
    }
  };
  const order = GROUPS.flatMap((g) => shown.filter((j) => groupOf(j) === g.key && (g.key !== 'rejected' || showRejected)));

  return (
    <>
      <button className="primary new-btn" onClick={onNew} data-automation-id="project-new">
        <Icon name="plus" /> 새로 만들기</button>
      <input className="search" placeholder="제목·글 번호 검색" aria-label="글 검색" value={q}
        onChange={(e) => setQ(e.target.value)} data-automation-id="project-search" />
      {err && <p className="error small pad">{err}</p>}
      <div className="jobs-list" role="listbox" aria-label="글 목록" onKeyDown={(e) => onKey(e, order)}>
        {jobs.length === 0 && <p className="muted small pad">아직 글이 없습니다. 위의 ‘새로 만들기’에 DC 링크를 넣으세요.</p>}
        {GROUPS.map((g) => {
          const items = shown.filter((j) => groupOf(j) === g.key);
          if (!items.length) return null;
          const hidden = g.key === 'rejected' && !showRejected;
          return (
            <div key={g.key} className="jgroup">
              <div className="jgroup-head">
                <span>{g.label} <span className="faint num">{items.length}</span></span>
                {g.key === 'rejected' && (
                  <button className="link small" onClick={() => setShowRejected(!showRejected)}>{hidden ? '보기' : '숨기기'}</button>)}
              </div>
              {!hidden && items.map((j) => {
                const on = j.id === selected;
                return (
                  <div key={j.id} role="option" aria-selected={on} tabIndex={on ? 0 : -1}
                    className={on ? 'jrow on' : 'jrow'} onClick={() => onSelect(j.id)}
                    onFocus={() => { if (!on) onSelect(j.id); }} data-automation-id={`job-${j.id}`}>
                    <div className="jrow-title" title={jobName(j)}>{jobName(j)}</div>
                    <div className="jrow-sub small">
                      <span className={TONE[g.key]}>{statusText(j)}</span>
                      <span className="faint num"> · #{j.id}</span>
                    </div>
                    {on && j.error && <div className="small muted jrow-err">{j.error}</div>}
                    {on && (
                      <div className="jrow-acts" onClick={(e) => e.stopPropagation()}>
                        {j.status === 'blocked' && (
                          <button className="small-btn" disabled={busy} data-automation-id={`job-force-${j.id}`}
                            onClick={() => act(() => api(`/api/jobs/${j.id}/force`, {}))}>그래도 만들기</button>)}
                        {j.error && j.status !== 'rejected' && j.status !== 'blocked' && (
                          <button className="small-btn" disabled={busy} data-automation-id={`job-retry-${j.id}`}
                            onClick={() => act(() => api(`/api/jobs/${j.id}/retry`, {}))}>이 단계부터 다시</button>)}
                        {(j.error || j.status === 'blocked') && (
                          <button className="small-btn" onClick={onShowLog} data-automation-id={`job-log-${j.id}`}>로그 보기</button>)}
                        <button className="small-btn danger" disabled={busy} onClick={() => setToDelete(j)}
                          data-automation-id={`job-delete-${j.id}`}>삭제</button>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          );
        })}
      </div>
      <div className="tasks-box" data-automation-id="queue-tasks">
        <div className="tasks-head small">
          <span className="muted">작업 대기열{worker ? ` · 실행 ${worker.current ? 1 : 0} · 대기 ${worker.pending.length}` : ''}</span>
          <button className="small-btn" disabled={busy} data-automation-id="queue-pause"
            onClick={() => act(() => api(`/api/worker/pause?paused=${!paused}`, {}))}>{paused ? '다시 시작' : '일시정지'}</button>
        </div>
        {worker?.current && (
          <div className="task small">
            <span className="state-warn">{worker.cancelling ? '멈추는 중' : '실행 중'}</span>
            <span className="grow" title={worker.current.label}>{worker.current.label}</span>
            <button className="small-btn danger" disabled={busy || worker.cancelling} data-automation-id="queue-cancel-current"
              onClick={() => act(() => api(`/api/worker/cancel?task_id=${worker.current?.id ?? ''}`, {}))}>취소</button>
          </div>
        )}
        {worker?.pending.map((t) => (
          <div key={t.id} className="task small">
            <span className="muted">대기</span>
            <span className="grow" title={t.label}>{t.label}</span>
            <button className="small-btn" disabled={busy} data-automation-id={`queue-remove-${t.id}`}
              onClick={() => act(() => apiDelete(`/api/worker/tasks/${t.id}`))}>빼기</button>
          </div>
        ))}
      </div>
      <Dialog open={toDelete !== null} title="이 작업을 지울까요?" onDismiss={() => setToDelete(null)}
        automationId="job-delete-dialog">
        {toDelete && (<>
          <p><b>{jobName(toDelete)}</b></p>
          <p className="muted">목록에서 빠지고 중간 파일(음성·미리보기)도 지웁니다. 다 만든 영상 파일은 남습니다.
            지금 이 작업을 처리하는 중이면 멈춥니다. 같은 글은 다시 넣으면 새로 만들 수 있습니다.</p>
          <div className="actions">
            <button onClick={() => setToDelete(null)}>그대로 두기</button>
            <button className="primary" data-automation-id="job-delete-confirm" onClick={() => {
              const j = toDelete; setToDelete(null);
              if (j) act(() => apiDelete(`/api/jobs/${j.id}`));
            }}>삭제</button>
          </div>
        </>)}
      </Dialog>
    </>
  );
}
