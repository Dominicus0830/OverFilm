import React, {useEffect, useRef, useState} from 'react';
import {api, Event, Job} from '../api';
import {Switch} from '../ui/Switch';
import {STAGES, eventOf, failedStage, isBad, jobName} from './jobs';

type ServerLog = {path: string | null; lines: string[]};

const LEVEL_KO = (e: Event) => (e.level === 'error' || (e.type === 'task' && e.state === 'failed') ? '오류'
  : isBad(e) ? '경고' : '정보');
const STATE_KO: Record<string, string> = {start: '시작', done: '끝', failed: '실패', cancelled: '취소됨', cancelling: '멈추는 중'};

function line(e: Event): string {
  if (e.type === 'task') return `${e.label ?? ''} ${STATE_KO[e.state ?? ''] ?? e.state ?? ''}${e.message ? `: ${e.message}` : ''}`;
  if (e.type === 'queue') return `대기열: 실행 ${e.current ? 1 : 0} · 대기 ${e.pending ?? 0}`;
  return [e.label, e.message ?? e.state].filter(Boolean).join(' ') || e.type;
}

/**
 * 아래 '로그' 탭(제목 줄 도구 + 내용을 돌려주는 훅): 작업 기록(실시간 이벤트)과 서버 기록(창 모드의 서버 로그 파일)을 한곳에.
 * 왼쪽은 고른 글의 단계 목록(실패한 단계 표시), 오른쪽은 기록. 필터: 고른 글만 · 오류·경고만.
 */
export function useLogPanel({events, job, active, reload}: {
  events: Event[]; job: Job | undefined; active: boolean; reload: () => void;
}): {tools: React.ReactNode; body: React.ReactNode} {
  const [source, setSource] = useState<'events' | 'server'>('events');
  const [onlyJob, setOnlyJob] = useState(false);
  const [onlyBad, setOnlyBad] = useState(false);
  const [follow, setFollow] = useState(true);
  const [server, setServer] = useState<ServerLog | null>(null);
  const [msg, setMsg] = useState('');
  const box = useRef<HTMLDivElement>(null);

  const loadServer = () => api<ServerLog>('/api/server-log').then((l) => { setServer(l); setMsg(''); })
    .catch((e) => setMsg((e as Error).message));
  useEffect(() => { if (active && source === 'server') loadServer(); }, [active, source]);
  // 실패가 나면 서버 기록도 새로 읽어 둔다(창 모드는 콘솔이 없어 오류가 여기에만 남는다)
  const lastBad = events.filter(isBad).length;
  useEffect(() => { if (active && source === 'server') loadServer(); }, [lastBad]);

  const shown = events.filter((e) => e.type !== 'jobs_changed' && (!onlyJob || eventOf(e, job)) && (!onlyBad || isBad(e)));
  const serverLines = (server?.lines ?? []).slice(-400)
    .filter((l) => !onlyBad || /error|warn|오류|경고|traceback|exception|실패/i.test(l));
  useEffect(() => {
    if (follow && box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [follow, shown.length, serverLines.length, source, active]);

  const copy = async () => {
    const text = source === 'events'
      ? shown.map((e) => `${e.time ?? ''} ${LEVEL_KO(e)} ${e.stage ?? ''} ${line(e)}`).join('\n')
      : serverLines.join('\n');
    try { await navigator.clipboard.writeText(text); setMsg('복사했습니다'); } catch { setMsg('복사하지 못했습니다'); }
  };
  const retry = async () => {
    if (!job) return;
    try {
      await api(job.status === 'blocked' ? `/api/jobs/${job.id}/force` : `/api/jobs/${job.id}/retry`, {});
      setMsg(job.status === 'blocked' ? '그래도 만들기 시작' : '이 단계부터 다시 시작'); reload();
    } catch (e) { setMsg((e as Error).message); }
  };
  const bad = job ? failedStage(job) : null;

  return {
    tools: (
      <>
        <span className="seg-tabs" role="group" aria-label="기록 종류">
          <button className={source === 'events' ? 'on' : ''} onClick={() => setSource('events')} data-automation-id="log-events">작업 기록</button>
          <button className={source === 'server' ? 'on' : ''} onClick={() => setSource('server')} data-automation-id="server-log-toggle">서버 기록</button>
        </span>
        {source === 'events' && <Switch checked={onlyJob} onChange={setOnlyJob} label="고른 글만" automationId="log-only-job" />}
        <Switch checked={onlyBad} onChange={setOnlyBad} label="오류·경고만" automationId="log-only-bad" />
        <Switch checked={follow} onChange={setFollow} label="실시간 따라가기" automationId="log-follow" />
        <button className="small-btn" onClick={copy} data-automation-id="log-copy">복사</button>
        {source === 'server' && <button className="small-btn" onClick={loadServer} data-automation-id="server-log-reload">다시 읽기</button>}
        <button className="small-btn" data-automation-id="server-log-folder"
          onClick={() => api('/api/server-log/open', {}).catch((e) => setMsg((e as Error).message))}>로그 폴더 열기</button>
      </>
    ),
    body: (
      <div className="logv">
        <div className="stages-col">
          <div className="small muted stages-title" title={job ? jobName(job) : undefined}>{job ? `#${job.id} 단계` : '글을 고르면 단계'}</div>
          <ol className="stages" aria-label="단계">
            {STAGES.map((s) => {
              const failed = bad === s.key;
              const done = !!job && s.done.includes(job.status) && !failed;
              return (
                <li key={s.key} className={failed ? 'bad' : done ? 'done' : ''}>
                  <span className="mark">{failed ? '실패' : done ? '완료' : ''}</span>{s.label}
                </li>
              );
            })}
          </ol>
          {job && bad && (
            <div className="stages-act">
              {job.error && <p className="small muted">{job.error}</p>}
              <button className="small-btn" onClick={retry} data-automation-id="log-retry">
                {job.status === 'blocked' ? '그래도 만들기' : '이 단계부터 다시'}</button>
            </div>
          )}
        </div>
        <div className="loglist" role="log" ref={box} aria-live="off">
          {source === 'events' && shown.length === 0 && <p className="faint small pad">{onlyJob || onlyBad ? '조건에 맞는 기록이 없습니다.' : '아직 기록이 없습니다.'}</p>}
          {source === 'events' && shown.map((e, i) => {
            const lv = LEVEL_KO(e);
            return (
              <div key={i} className={`lrow ${lv === '오류' ? 'error' : ''}`}>
                <span className="faint num">{e.time?.slice(11, 19)}</span>
                <span className={`lvl ${lv === '오류' ? 'error' : lv === '경고' ? 'warn' : 'info'}`}>{lv}</span>
                <span className="lmsg">{line(e)}</span>
              </div>
            );
          })}
          {source === 'server' && server && !server.path && <p className="muted small pad">로그 파일이 없습니다. 서버 출력은 실행한 콘솔 창에 나옵니다.</p>}
          {source === 'server' && server?.path && (<>
            <div className="faint small pad"><code>{server.path}</code></div>
            <pre className="server-log-text">{serverLines.join('\n') || '(비어 있음)'}</pre>
          </>)}
          {msg && <p className="small muted pad" role="status">{msg}</p>}
        </div>
      </div>
    ),
  };
}
