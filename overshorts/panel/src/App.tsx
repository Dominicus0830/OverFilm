import React, {useEffect, useRef, useState} from 'react';
import {Event, WorkerState, api, subscribe} from './api';
import {Home} from './screens/Home';
import {NewJob} from './screens/NewJob';
import {Queue} from './screens/Queue';
import {Review} from './screens/Review';
import {StyleMaker} from './screens/StyleMaker';
import {Settings} from './screens/Settings';
import {Voices} from './screens/Voices';
import {Dialog} from './ui/Dialog';
import {Icon, IconName} from './ui/Icon';

const SCREENS = ['홈', '새로 만들기', '작업 큐', '검수', '보이스', '스타일', '소스', '설정'] as const;
/** 아직 화면이 없는 메뉴: 메뉴에 '준비 중'으로 보이고, 누르면 지금 쓰는 방법을 알려 준다. */
const SOON: readonly string[] = ['소스'];
/** 메뉴마다 붙는 선형 아이콘(편집기·OverFilmSubs 와 같은 그림). */
const NAV_ICON: Record<string, IconName> = {
  '홈': 'home', '새로 만들기': 'plus', '작업 큐': 'list', '검수': 'eye', '보이스': 'mic', '스타일': 'adjust', '소스': 'folder', '설정': 'gear',
};
export type Screen = (typeof SCREENS)[number];

export function App() {
  const [screen, setScreen] = useState<Screen>('홈');
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
    <div className="app">
      <nav className="side" aria-label="화면">
        <div className="brand">OverShorts</div>
        {SCREENS.map((s) => (
          <button key={s} className={s === screen ? 'nav on' : 'nav'} onClick={() => setScreen(s)}
            aria-current={s === screen ? 'page' : undefined} data-automation-id={`nav-${s}`}>
            <Icon name={NAV_ICON[s] ?? 'list'} /><span className="nav-label">{s}</span>
            {SOON.includes(s) && <span className="soon">준비 중</span>}</button>
        ))}
        <button className="nav theme" onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')}>
          <Icon name={theme === 'dark' ? 'sun' : 'moon'} /><span className="nav-label">{theme === 'dark' ? '라이트 테마' : '다크 테마'}</span>
        </button>
      </nav>
      {/* 화면을 바꿀 때 내용만 0.12초 옅게 나타난다(움직임 줄이기 설정이면 없음) */}
      <main className="main" key={screen}>
        {screen === '홈' && <Home tick={tick} go={setScreen} events={events} />}
        {screen === '새로 만들기' && <NewJob go={setScreen} />}
        {screen === '작업 큐' && <Queue tick={tick} events={events} />}
        {screen === '검수' && <Review tick={tick} go={setScreen} />}
        {screen === '스타일' && <StyleMaker />}
        {screen === '설정' && <Settings />}
        {screen === '보이스' && <Voices tick={tick} />}
        {SOON.includes(screen) && (
          <section className="empty">
            <h1>{screen}</h1>
            <p>이 화면은 아직 준비 중입니다. 지금은 갤러리 주소를 <b>새로 만들기</b>에 넣거나,
              <code>config.yaml</code> 의 수집 항목을 고쳐서 씁니다.</p>
            <div className="actions"><button onClick={() => setScreen('새로 만들기')}>새로 만들기로 가기</button>
              <button className="link" onClick={() => setScreen('설정')}>설정 열기</button></div>
          </section>
        )}
      </main>
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
    </div>
  );
}
