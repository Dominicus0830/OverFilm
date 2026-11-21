import React, {useCallback, useEffect, useMemo, useState} from 'react';
import {api, Event, Job, Status, WorkerState} from '../api';
import type {Place} from '../nav';
import {NewJob} from '../screens/NewJob';
import {Review, ReviewSlots} from '../screens/Review';
import {Settings} from '../screens/Settings';
import {Voices} from '../screens/Voices';
import {Dialog} from '../ui/Dialog';
import {Icon} from '../ui/Icon';
import {Pane, Splitter, openSection, useStored} from '../ui/Pane';
import {groupOf, jobName, statusText} from './jobs';
import {useLogPanel} from './LogPanel';
import {ProjectPanel} from './ProjectPanel';
import {StylePanel} from './StylePanel';
import './workspace.css';

const clamp = (v: unknown, lo: number, hi: number, d: number) =>
  typeof v === 'number' && Number.isFinite(v) ? Math.min(hi, Math.max(lo, v)) : d;

/**
 * Premiere Pro 식 작업 공간: 화면 하나에 패널 다섯.
 *   왼쪽 프로젝트(글 목록·대기열) · 가운데 프로그램 모니터(미리보기·승인) · 오른쪽 속성(대본·보이스·스타일…)
 *   · 아래 타임라인/로그. 전역 설정은 오른쪽 위 톱니, 보이스 관리는 그 옆.
 * 패널은 접고(제목 줄 화살표) 크기를 바꾼다(사이 막대). 크기·접힘은 다음에도 그대로.
 */
export function Workspace({events, tick, theme, setTheme, refresh}: {
  events: Event[]; tick: number; theme: string; setTheme: (t: string) => void; refresh: () => void;
}) {
  const [jobs, setJobs] = useState<Job[]>([]);
  const [worker, setWorker] = useState<WorkerState | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [err, setErr] = useState('');
  const [notice, setNotice] = useState('');
  const [selected, setSelected] = useState<number | null>(null);
  const [reviewIds, setReviewIds] = useState<number[]>([]);
  const [sheet, setSheet] = useState<'new' | 'settings' | 'voices' | null>(null);
  const [tab, setTab] = useStored<'timeline' | 'log'>('ws.tab', 'timeline');
  const [sizes, setSizes] = useStored('ws.sizes', {left: 288, right: 380, bottom: 248});
  const [fold, setFold] = useStored<Record<string, boolean>>('ws.fold', {});
  const [slots, setSlots] = useState<ReviewSlots>({monitor: null, props: null, timeline: null});
  const monitorRef = useCallback((el: HTMLDivElement | null) => setSlots((s) => (s.monitor === el ? s : {...s, monitor: el})), []);
  const propsRef = useCallback((el: HTMLDivElement | null) => setSlots((s) => (s.props === el ? s : {...s, props: el})), []);
  const timelineRef = useCallback((el: HTMLDivElement | null) => setSlots((s) => (s.timeline === el ? s : {...s, timeline: el})), []);

  const left = clamp(sizes.left, 220, 520, 288);
  const right = clamp(sizes.right, 300, 640, 380);
  const bottom = clamp(sizes.bottom, 120, 640, 248);

  const load = useCallback(() => {
    api<Job[]>('/api/jobs').then((j) => { setJobs(j); setErr(''); }).catch((e) => setErr((e as Error).message));
    api<WorkerState>('/api/worker').then(setWorker).catch(() => undefined);
    api<Status>('/api/status').then(setStatus).catch(() => undefined);
  }, []);
  const last = events[events.length - 1];
  useEffect(load, [tick, load, last?.type === 'queue' || last?.type === 'task' ? events.length : 0]);

  // 처음엔(또는 고른 글이 지워지면) 검수 대기 첫 글, 없으면 맨 위 글
  useEffect(() => {
    if (selected !== null && jobs.some((j) => j.id === selected)) return;
    const first = jobs.find((j) => groupOf(j) === 'review') ?? jobs[0];
    setSelected(first ? first.id : null);
  }, [jobs, selected]);

  const job = jobs.find((j) => j.id === selected);
  const inReview = selected !== null && reviewIds.includes(selected);
  const onItems = useCallback((ids: number[]) => setReviewIds(ids), []);
  const onNotice = useCallback((m: string) => setNotice(m), []);
  const log = useLogPanel({events, job, active: tab === 'log' && !fold.bottom, reload: () => { load(); refresh(); }});
  const lastBad = useMemo(() => events.filter((e) => e.type === 'task' && e.state === 'failed').length, [events]);

  const go = (where: Place) => {
    if (where === 'style') { setSheet(null); setFold((f) => ({...f, props: false})); setTimeout(() => openSection('style'), 50); }
    else if (where === 'close') { setSheet(null); load(); }
    else setSheet(where);
  };
  const toggle = (k: string) => setFold((f) => ({...f, [k]: !f[k]}));
  const cols = `${fold.project ? 34 : left}px 4px minmax(320px, 1fr) 4px ${fold.props ? 34 : right}px`;
  const rows = `minmax(240px, 1fr) 4px ${fold.bottom ? 31 : bottom}px`;

  const running = worker?.current?.label;
  return (
    <div className="ws">
      <header className="ws-bar">
        <span className="brand-s">OverShorts</span>
        <span className="ws-status small" role="status">
          {running ? <><span className="state-warn">실행 중</span> {running}</> : <span className="muted">쉬는 중</span>}
          {worker && worker.pending.length > 0 && <span className="muted"> · 대기 {worker.pending.length}</span>}
          {status && (<>
            <span className="sep">|</span>
            <span className={status.claude.limited ? 'state-warn' : 'muted'}>Claude {status.claude.limited ? '한도 걸림' : '정상'}</span>
            <span className="sep">|</span>
            <span className={status.budget_warnings.length ? 'state-warn' : 'muted'}>
              크롤링 {status.budget.hour_used}/{status.budget.hour_limit} (이번 시간) · {status.budget.day_used}/{status.budget.day_limit} (오늘)</span>
          </>)}
          {notice && <><span className="sep">|</span><span>{notice}</span></>}
          {err && <><span className="sep">|</span><span className="error">{err}</span></>}
        </span>
        <span className="ws-right">
          <button className="icon-btn text" onClick={() => setSheet('voices')} data-automation-id="ws-voices" title="목소리 관리 (복제·기본 목소리)">
            <Icon name="mic" /><span>보이스</span></button>
          <button className="icon-btn" aria-label={theme === 'dark' ? '라이트 테마' : '다크 테마'} title="테마"
            onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')} data-automation-id="ws-theme">
            <Icon name={theme === 'dark' ? 'sun' : 'moon'} /></button>
          <button className="icon-btn" aria-label="설정" title="설정 (전역)" onClick={() => setSheet('settings')}
            data-automation-id="ws-settings"><Icon name="gear" /></button>
        </span>
      </header>

      <div className="ws-grid" style={{gridTemplateColumns: cols, gridTemplateRows: rows}}>
        <Pane id="project" title="프로젝트" folded={!!fold.project} onFold={() => toggle('project')} className="area-project"
          tools={<span className="faint small num">{jobs.length}개</span>}>
          <ProjectPanel jobs={jobs} worker={worker} selected={selected} onSelect={setSelected}
            onNew={() => setSheet('new')} onShowLog={() => { setTab('log'); setFold((f) => ({...f, bottom: false})); }}
            reload={load} />
        </Pane>
        <Splitter dir="x" label="프로젝트 패널 너비" onDrag={(d) => setSizes((s) => ({...s, left: clamp(s.left + d, 220, 520, 288)}))} />

        <Pane id="monitor" className="area-monitor"
          title={job ? <>프로그램: <b>{jobName(job)}</b></> : '프로그램'}
          tools={job ? <span className="faint small">#{job.id} · {statusText(job)}</span> : undefined}>
          <div className="slot" ref={monitorRef} hidden={!inReview} />
          {!inReview && <JobState job={job} onNew={() => setSheet('new')} reload={load}
            onLog={() => { setTab('log'); setFold((f) => ({...f, bottom: false})); }} />}
        </Pane>
        <Splitter dir="x" label="속성 패널 너비" onDrag={(d) => setSizes((s) => ({...s, right: clamp(s.right - d, 300, 640, 380)}))} />

        <Pane id="props" title={fold.props ? '속성' : <>속성{job ? <span className="faint"> · #{job.id}</span> : null}</>}
          folded={!!fold.props} onFold={() => toggle('props')} className="area-props" right>
          <div className="slot" ref={propsRef} hidden={!inReview} />
          {!inReview && job && <p className="muted small pad">이 글은 {statusText(job)} 상태라 대본·보이스는 검수 대기가 되면 여기에 나옵니다.</p>}
          <StylePanel tick={tick} />
        </Pane>

        <Splitter dir="y" label="아래 패널 높이" onDrag={(d) => setSizes((s) => ({...s, bottom: clamp(s.bottom - d, 120, 640, 248)}))} />

        <section className={`pane area-bottom ${fold.bottom ? 'folded' : ''}`} data-automation-id="pane-bottom">
          <header className="pane-head tabs" role="tablist">
            <button className="fold" onClick={() => toggle('bottom')} aria-expanded={!fold.bottom}
              aria-label={fold.bottom ? '펼치기' : '접기'} data-automation-id="pane-bottom-fold">
              <span className={fold.bottom ? 'rot-up' : ''}><Icon name="chevron" /></span></button>
            <button role="tab" aria-selected={tab === 'timeline'} className={tab === 'timeline' ? 'tab on' : 'tab'}
              onClick={() => { setTab('timeline'); setFold((f) => ({...f, bottom: false})); }} data-automation-id="tab-timeline">타임라인</button>
            <button role="tab" aria-selected={tab === 'log'} className={tab === 'log' ? 'tab on' : 'tab'}
              onClick={() => { setTab('log'); setFold((f) => ({...f, bottom: false})); }} data-automation-id="tab-log">
              로그{lastBad > 0 && <span className="error num"> {lastBad}</span>}</button>
            {!fold.bottom && <span className="pane-tools">{tab === 'log' ? log.tools : null}</span>}
          </header>
          <div className="pane-body" hidden={fold.bottom}>
            <div className="slot" ref={timelineRef} hidden={tab !== 'timeline' || !inReview} />
            {tab === 'timeline' && !inReview && (
              <p className="muted small pad">검수 대기 글을 고르면 장면·자막·음성이 시간 순서로 보입니다. 장면을 누르면 그 장면 대본으로 갑니다.</p>)}
            {tab === 'log' && log.body}
          </div>
        </section>
      </div>

      {/* 검수(미리보기·대본·타임라인)는 상태를 하나로 갖고 세 자리에 그린다 */}
      <Review tick={tick} jobId={selected} onPick={setSelected} onItems={onItems} onNotice={onNotice} slots={slots} />

      <Dialog open={sheet === 'new'} title="새로 만들기" onDismiss={() => setSheet(null)} automationId="new-dialog" size="wide">
        <NewJob go={go} />
      </Dialog>
      <Dialog open={sheet === 'settings'} title="설정" onDismiss={() => setSheet(null)} automationId="settings-dialog" size="sheet">
        <button className="sheet-close" onClick={() => setSheet(null)} aria-label="닫기" data-automation-id="sheet-close">닫기</button>
        <Settings />
      </Dialog>
      <Dialog open={sheet === 'voices'} title="보이스" onDismiss={() => setSheet(null)} automationId="voices-dialog" size="sheet">
        <button className="sheet-close" onClick={() => setSheet(null)} aria-label="닫기" data-automation-id="sheet-close">닫기</button>
        <Voices tick={tick} />
      </Dialog>
    </div>
  );
}

/** 모니터: 고른 글이 검수 대기가 아니면 지금 상태와 할 수 있는 일. */
function JobState({job, onNew, onLog, reload}: {job: Job | undefined; onNew: () => void; onLog: () => void; reload: () => void}) {
  const [msg, setMsg] = useState('');
  if (!job) {
    return (
      <div className="monitor-empty">
        <p>아직 글이 없습니다.</p>
        <p className="muted small">DC 갤러리 링크나 게시물 링크를 넣으면 수집 → 대본 → 음성 → 미리보기까지 만들고 여기서 검수합니다.</p>
        <button className="primary" onClick={onNew} data-automation-id="monitor-new">새로 만들기</button>
      </div>
    );
  }
  const g = groupOf(job);
  const run = (path: string) => api(path, {}).then(() => { setMsg('다시 시작했습니다'); reload(); })
    .catch((e) => setMsg((e as Error).message));
  return (
    <div className="monitor-empty" data-automation-id="monitor-state">
      <p className={g === 'fail' ? 'error' : g === 'run' ? 'state-warn' : undefined}><b>{statusText(job)}</b></p>
      {job.error && <p className="small muted">{job.error}</p>}
      {g === 'run' && <p className="muted small">만드는 중입니다. 미리보기가 준비되면 여기서 바로 볼 수 있습니다.</p>}
      {g === 'wait' && <p className="muted small">대기열에서 차례를 기다립니다.</p>}
      {g === 'done' && <p className="muted small">다 만든 영상은 설정의 '다 만든 영상' 폴더에 있습니다.</p>}
      {g === 'rejected' && <p className="muted small">폐기한 글입니다. 같은 링크를 다시 넣으면 새로 만듭니다.</p>}
      <div className="actions center">
        {job.status === 'blocked' && <button className="primary" onClick={() => run(`/api/jobs/${job.id}/force`)}>그래도 만들기</button>}
        {job.error && job.status !== 'blocked' && job.status !== 'rejected' && (
          <button className="primary" onClick={() => run(`/api/jobs/${job.id}/retry`)}>이 단계부터 다시</button>)}
        {g === 'fail' && <button onClick={onLog}>로그 보기</button>}
      </div>
      {msg && <p className="small muted" role="status">{msg}</p>}
    </div>
  );
}
