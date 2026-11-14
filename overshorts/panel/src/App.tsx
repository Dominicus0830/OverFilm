import React, {useEffect, useState} from 'react';
import {Event, subscribe} from './api';
import {Home} from './screens/Home';
import {NewJob} from './screens/NewJob';
import {Queue} from './screens/Queue';
import {Review} from './screens/Review';

const SCREENS = ['홈', '새로 만들기', '작업 큐', '검수', '보이스', '스타일', '소스', '설정'] as const;
export type Screen = (typeof SCREENS)[number];

export function App() {
  const [screen, setScreen] = useState<Screen>('홈');
  const [events, setEvents] = useState<Event[]>([]);
  const [tick, setTick] = useState(0); // 작업이 바뀌면 화면들이 다시 읽는다
  const [theme, setTheme] = useState<string>(() => {
    try { return localStorage.getItem('theme') || 'dark'; } catch { return 'dark'; }
  });
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    try { localStorage.setItem('theme', theme); } catch { /* 저장 못 해도 됨 */ }
  }, [theme]);
  useEffect(() => subscribe((e) => {
    setEvents((old) => [...old.slice(-199), e]);
    if (e.type === 'jobs_changed' || e.type === 'task') setTick((t) => t + 1);
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
            data-automation-id={`nav-${s}`}>{s}</button>
        ))}
        <button className="nav theme" onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')}>
          {theme === 'dark' ? '라이트 테마' : '다크 테마'}
        </button>
      </nav>
      <main className="main">
        {screen === '홈' && <Home tick={tick} go={setScreen} events={events} />}
        {screen === '새로 만들기' && <NewJob go={setScreen} />}
        {screen === '작업 큐' && <Queue tick={tick} events={events} />}
        {screen === '검수' && <Review tick={tick} go={setScreen} />}
        {['보이스', '스타일', '소스', '설정'].includes(screen) && (
          <section className="empty">
            <h1>{screen}</h1>
            <p>이 화면은 다음 단계(M8)에서 만듭니다. 지금은 <code>config.yaml</code> 로 설정합니다.</p>
          </section>
        )}
      </main>
    </div>
  );
}
