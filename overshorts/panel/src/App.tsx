import React, {useEffect, useRef, useState} from 'react';
import {Event, WorkerState, api, subscribe} from './api';
import {Workspace} from './workspace/Workspace';
import {Dialog} from './ui/Dialog';

export function App() {
  const [events, setEvents] = useState<Event[]>([]);
  const [tick, setTick] = useState(0); // 작업이 바뀌면 화면들이 다시 읽는다
  const [limitAsk, setLimitAsk] = useState<string | null>(null);
  const [remember, setRemember] = useState(false);
  const [theme, setTheme] = useState<string>(() => {
    try { return localStorage.getItem('theme') || 'dark'; } catch { return 'dark'; }
  });
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    try { localStorage.setItem('theme', theme); } catch { /* 저장 못 해도 됨 */ }
  }, [theme]);
  // 작업이 도는 중에 창을 닫으려 하면 묻는다 (창 모드는 창을 닫으면 서버도 꺼져 작업이 멈춘다)
  const working = useRef(false);
  useEffect(() => {
    api<WorkerState>('/api/worker')
      .then((w) => { working.current = w.current !== null || w.pending.length > 0; })
      .catch(() => { /* 모르면 묻지 않는다 */ });
    const onLeave = (ev: BeforeUnloadEvent) => {
      if (!working.current) return;
      ev.preventDefault();
      ev.returnValue = '작업이 도는 중입니다. 창을 닫으면 작업이 멈춥니다.';
    };
    window.addEventListener('beforeunload', onLeave);
    return () => window.removeEventListener('beforeunload', onLeave);
  }, []);
  // 창을 다시 열어도 로그가 비지 않게: 서버가 기억하는 최근 기록부터
  useEffect(() => {
    let live = true; // 개발 모드처럼 두 번 붙었다 떨어져도 한 번만 넣는다
    api<Event[]>('/api/log').then((old) => {
      if (live && Array.isArray(old)) setEvents((now) => [...old.slice(-200), ...now].slice(-200));
    }).catch(() => undefined);
    return () => { live = false; };
  }, []);
  useEffect(() => subscribe((e) => {
    if (e.type === 'queue') working.current = Boolean(e.current) || (e.pending ?? 0) > 0;
    setEvents((old) => [...old.slice(-199), e]);
    if (e.type === 'jobs_changed' || e.type === 'task') setTick((t) => t + 1);
    if (e.type === 'confirm' && e.kind === 'llm_limit') setLimitAsk(e.message ?? 'Claude 한도에 걸렸습니다');
    if (e.type === 'alert' && 'Notification' in window && Notification.permission === 'granted') {
      new Notification('OverShorts', {body: e.message ?? ''});
    }
  }), []);

  return (
    <>
      <Workspace events={events} tick={tick} theme={theme} setTheme={setTheme} refresh={() => setTick((t) => t + 1)} />
      <Dialog open={limitAsk !== null} title="Claude 한도에 걸렸습니다" automationId="limit-dialog">
        <p>{limitAsk}</p>
        <label className="check"><input type="checkbox" checked={remember}
          onChange={(e) => setRemember(e.target.checked)} /> 다시 묻지 않기</label>
        <div className="actions">
          <button data-automation-id="limit-wait" onClick={() => {
            api('/api/llm/limit', {action: 'wait', remember}).catch(() => undefined); setLimitAsk(null);
          }}>대기</button>
          <button className="primary" data-automation-id="limit-switch" onClick={() => {
            api('/api/llm/limit', {action: 'switch', remember}).catch(() => undefined); setLimitAsk(null);
          }}>로컬 LLM 으로 전환</button>
        </div>
      </Dialog>
    </>
  );
}
