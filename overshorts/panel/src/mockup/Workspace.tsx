import React, {useCallback, useRef, useState} from 'react';
import {Icon} from '../ui/Icon';
import {Switch} from '../ui/Switch';
import {Dialog} from '../ui/Dialog';

/*
 * Premiere Pro 식 작업 공간 시안(가짜 데이터). 화면 하나에 패널 다섯 개:
 *   왼쪽 프로젝트(글 목록 = 작업 큐) · 가운데 프로그램 모니터 · 오른쪽 속성 · 아래 타임라인/로그.
 * 패널은 제목 줄의 화살표로 접고, 사이 막대를 끌어 크기를 바꾼다. 전역 설정은 오른쪽 위 톱니 하나.
 * 승인 전 시안이라 서버를 부르지 않는다.
 */

type Job = {id: number; title: string; gall: string; kind: string; state: 'review' | 'run' | 'wait' | 'done' | 'fail';
  note: string; progress?: number; secs?: number};

const JOBS: Job[] = [
  {id: 412, title: '햄스터 신규 스킨 유출 떴다는데 이거 진짜냐', gall: '오버워치2 마갤', kind: '이슈', state: 'review', note: '검수 대기', secs: 41.2},
  {id: 409, title: '브론즈 3에서 탈출하는 법 알려준다', gall: '오버워치2 마갤', kind: '정보', state: 'review', note: '검수 대기', secs: 36.8},
  {id: 415, title: '오늘 경쟁 5연패한 썰 푼다', gall: '옵갤', kind: '옵갤', state: 'run', note: '음성 만드는 중', progress: 0.62},
  {id: 416, title: '신규 영웅 패치노트 요약', gall: '오버워치2 마갤', kind: '정보', state: 'wait', note: '대기 1번'},
  {id: 417, title: '탱커 3황 체감 어떰?', gall: '옵갤', kind: '게임', state: 'wait', note: '대기 2번'},
  {id: 403, title: '루시우 벽타기 매드무비 반응 모음', gall: '오버워치2 마갤', kind: '이슈', state: 'fail', note: '렌더 실패'},
  {id: 398, title: '시즌 20 밴율 정리', gall: '오버워치2 마갤', kind: '정보', state: 'done', note: '완료 · 어제', secs: 44.0},
  {id: 395, title: '아나 재우기 장인 등장', gall: '옵갤', kind: '옵갤', state: 'done', note: '완료 · 어제', secs: 32.5},
];

const GROUPS: {state: Job['state']; label: string}[] = [
  {state: 'review', label: '검수 대기'}, {state: 'run', label: '만드는 중'}, {state: 'wait', label: '대기'},
  {state: 'fail', label: '실패'}, {state: 'done', label: '완료'},
];

type Scene = {kind: string; label: string; secs: number};
const SCENES: Scene[] = [
  {kind: 'title', label: '제목', secs: 3.2}, {kind: 'media', label: '본문 미디어', secs: 8.0},
  {kind: 'body', label: '본문 글', secs: 12.4}, {kind: 'comments', label: '댓글 반응', secs: 13.6},
  {kind: 'outro', label: '마무리', secs: 4.0},
];
const TOTAL = SCENES.reduce((a, s) => a + s.secs, 0);

const LINES = [
  {part: '훅', text: '햄스터 신규 스킨이 유출됐다는 글이 올라왔습니다'},
  {part: '전개', text: '작성자는 테스트 서버에서 직접 봤다고 주장하는데요'},
  {part: '전개', text: '사진 속 스킨은 기계 갑옷에 금색 장식이 들어가 있습니다', flag: '원문과 다름'},
  {part: '전개', text: '다만 공식 발표는 아직 없는 상태입니다'},
];

type Log = {t: string; level: 'info' | 'warn' | 'error'; stage: string; msg: string; job: number};
const LOGS: Log[] = [
  {t: '23:40:02', level: 'info', stage: '수집', msg: '글 1648189 가져옴 (사진 3, 댓글 48)', job: 412},
  {t: '23:40:05', level: 'info', stage: '필터', msg: '통과 (유형: 이슈, 점수 0.82)', job: 412},
  {t: '23:40:31', level: 'info', stage: '대본', msg: 'A안 9줄, 예상 41초', job: 412},
  {t: '23:40:33', level: 'warn', stage: '대본', msg: '3번 줄이 원문에 없는 표현을 씀 (원문과 다름 표시)', job: 412},
  {t: '23:41:10', level: 'info', stage: '음성', msg: 'GPT-SoVITS (CPU) 9줄 완료, 38.6초', job: 412},
  {t: '23:41:12', level: 'info', stage: '자막 타이밍', msg: '단어 172개 맞춤', job: 412},
  {t: '23:41:40', level: 'info', stage: '미리보기', msg: '검수 대기로 옮김', job: 412},
  {t: '23:31:40', level: 'info', stage: '수집', msg: '글 1647902 가져옴 (영상 1, 댓글 61)', job: 403},
  {t: '23:31:44', level: 'info', stage: '필터', msg: '통과 (유형: 이슈, 점수 0.77)', job: 403},
  {t: '23:32:20', level: 'info', stage: '대본', msg: 'A안 8줄, 예상 38초', job: 403},
  {t: '23:33:02', level: 'info', stage: '음성', msg: 'GPT-SoVITS (CPU) 8줄 완료, 37.1초', job: 403},
  {t: '23:33:05', level: 'info', stage: '자막 타이밍', msg: '단어 158개 맞춤', job: 403},
  {t: '23:33:30', level: 'info', stage: '미리보기', msg: '사람 승인 (23:41)', job: 403},
  {t: '23:42:03', level: 'info', stage: '렌더', msg: '탭 2개로 렌더 시작 (1080×1920, 30fps)', job: 403},
  {t: '23:43:18', level: 'error', stage: '렌더', msg: '프레임 612에서 배경 영상 디코딩 실패: bg_03.mp4 (파일이 잘림)', job: 403},
  {t: '23:43:18', level: 'warn', stage: '렌더', msg: '작업을 실패로 표시함. 이 단계부터 다시 할 수 있습니다', job: 403},
  {t: '23:44:01', level: 'info', stage: '음성', msg: '6 / 10줄 만드는 중', job: 415},
];
const STAGES = ['수집', '필터', '대본', '음성', '자막 타이밍', '미리보기', '렌더'];

/** 사이 막대: 끌면 옆 패널 크기가 바뀐다(최소·최대 안에서). */
function Splitter({dir, onDrag, label}: {dir: 'x' | 'y'; onDrag: (d: number) => void; label: string}) {
  const last = useRef<number | null>(null);
  return (
    <div className={`split split-${dir}`} role="separator" aria-orientation={dir === 'x' ? 'vertical' : 'horizontal'}
      aria-label={label} tabIndex={0}
      onKeyDown={(e) => {
        const k: Record<string, number> = dir === 'x' ? {ArrowLeft: -16, ArrowRight: 16} : {ArrowUp: -16, ArrowDown: 16};
        const d = k[e.key];
        if (d) { e.preventDefault(); onDrag(d); }
      }}
      onPointerDown={(e) => { last.current = dir === 'x' ? e.clientX : e.clientY; e.currentTarget.setPointerCapture(e.pointerId); }}
      onPointerMove={(e) => {
        if (last.current === null) return;
        const now = dir === 'x' ? e.clientX : e.clientY;
        onDrag(now - last.current); last.current = now;
      }}
      onPointerUp={() => { last.current = null; }} />
  );
}

/** 패널 틀: 제목 줄(이름 + 오른쪽 도구 + 접기) 과 내용. */
function Panel({id, title, tools, folded, onFold, children, className}: {
  id: string; title: React.ReactNode; tools?: React.ReactNode; folded: boolean; onFold?: () => void;
  children: React.ReactNode; className?: string;
}) {
  return (
    <section className={`pane ${className ?? ''} ${folded ? 'folded' : ''}`} data-automation-id={`pane-${id}`}>
      <header className="pane-head">
        {onFold ? <button className="fold" onClick={onFold} aria-expanded={!folded} aria-label={folded ? '펼치기' : '접기'}
          data-automation-id={`pane-${id}-fold`}><span className={folded ? 'rot' : ''}><Icon name="chevron" /></span></button> : <span className="fold-gap" />}
        <span className="pane-title">{title}</span>
        {!folded && <span className="pane-tools">{tools}</span>}
      </header>
      {!folded && <div className="pane-body">{children}</div>}
    </section>
  );
}

function Section({title, open: initial = true, children, extra}: {title: string; open?: boolean; children: React.ReactNode; extra?: React.ReactNode}) {
  const [open, setOpen] = useState(initial);
  return (
    <div className="prop">
      <div className="prop-head">
        <button className="prop-toggle" onClick={() => setOpen(!open)} aria-expanded={open}>
          <Icon name="chevron" size={12} className={open ? 'turn open' : 'turn'} />{title}
        </button>
        {extra}
      </div>
      {open && <div className="prop-body">{children}</div>}
    </div>
  );
}

const fmt = (s: number) => `${Math.floor(s / 60)}:${(s % 60).toFixed(1).padStart(4, '0')}`;

export function Workspace({view}: {view: string}) {
  const [left, setLeft] = useState(288);
  const [right, setRight] = useState(360);
  const [bottom, setBottom] = useState(view === 'log' ? 300 : 248);
  const [fold, setFold] = useState<Record<string, boolean>>(
    view === 'collapsed' ? {project: true, props: true} : {});
  const [sel, setSel] = useState(view === 'log' ? 403 : 412);
  const [scene, setScene] = useState(2);
  const [tab, setTab] = useState<'timeline' | 'log'>(view === 'log' ? 'log' : 'timeline');
  const [onlyBad, setOnlyBad] = useState(view === 'log');
  const [logJob, setLogJob] = useState<'sel' | 'all'>('sel');
  const [follow, setFollow] = useState(true);
  const [learn, setLearn] = useState(view === 'learn' || view === 'passed');
  const [passed, setPassed] = useState(view === 'passed');
  const [learnTo, setLearnTo] = useState<'fill' | 'new'>('fill');
  const [source, setSource] = useState(true);
  const toggle = useCallback((k: string) => setFold((f) => ({...f, [k]: !f[k]})), []);
  const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

  const job = JOBS.find((j) => j.id === sel) ?? JOBS[0];
  const startAt = SCENES.slice(0, scene).reduce((a, s) => a + s.secs, 0);
  const head = startAt + 2.6;
  const logs = LOGS.filter((l) => (logJob === 'all' || l.job === sel) && (!onlyBad || l.level !== 'info'));
  const failed = job.state === 'fail';
  const cols = `${fold.project ? 32 : left}px 4px 1fr 4px ${fold.props ? 32 : right}px`;
  const rows = `1fr 4px ${fold.bottom ? 32 : bottom}px`;

  return (
    <div className="ws">
      <header className="ws-bar">
        <span className="brand-s">OverShorts</span>
        <span className="ws-status muted small">
          <span className="state-warn">실행 중</span> 오늘 경쟁 5연패한 썰 푼다 · 음성 62% <span className="sep">|</span>
          Claude 한도 정상 <span className="sep">|</span> 크롤링 3/20 (이번 시간)
        </span>
        <span className="ws-right">
          <button className="icon-btn" aria-label="라이트 테마" title="테마"><Icon name="sun" /></button>
          <button className="icon-btn" aria-label="설정" title="설정 (전역)" data-automation-id="ws-settings"><Icon name="gear" /></button>
        </span>
      </header>

      <div className="ws-grid" style={{gridTemplateColumns: cols, gridTemplateRows: rows}}>
        {/* 프로젝트 */}
        <Panel id="project" title="프로젝트" folded={!!fold.project} onFold={() => toggle('project')}
          className="area-project"
          tools={<span className="faint small num">{JOBS.length}개</span>}>
          <button className="primary new-btn" data-automation-id="project-new"><Icon name="plus" /> 새로 만들기</button>
          <input className="search" placeholder="제목·갤러리 검색" aria-label="글 검색" />
          <div className="jobs-list" role="listbox" aria-label="글 목록">
            {GROUPS.map((g) => {
              const items = JOBS.filter((j) => j.state === g.state);
              if (!items.length) return null;
              return (
                <div key={g.state} className="jgroup">
                  <div className="jgroup-head">{g.label} <span className="faint num">{items.length}</span></div>
                  {items.map((j) => (
                    <div key={j.id} role="option" aria-selected={j.id === sel} tabIndex={0}
                      className={j.id === sel ? 'jrow on' : 'jrow'} onClick={() => setSel(j.id)}
                      data-automation-id={`job-${j.id}`}>
                      <div className="jrow-title">{j.title}</div>
                      <div className="jrow-sub small">
                        <span className={j.state === 'fail' ? 'error' : j.state === 'review' ? 'state-ok' : j.state === 'run' ? 'state-warn' : 'muted'}>{j.note}</span>
                        <span className="faint"> · {j.kind}{j.secs ? ` · ${j.secs.toFixed(0)}초` : ''}</span>
                      </div>
                      {j.progress !== undefined && <div className="bar thin"><div className="fill" style={{width: `${j.progress * 100}%`}} /></div>}
                      {j.id === sel && (
                        <div className="jrow-acts">
                          {j.state === 'run' && <button className="small-btn danger">취소</button>}
                          {j.state === 'wait' && <button className="small-btn">대기에서 빼기</button>}
                          {j.state === 'fail' && <button className="small-btn" onClick={() => setTab('log')}>로그 보기</button>}
                          <button className="small-btn">폴더</button>
                          <button className="small-btn danger">삭제</button>
                        </div>
                      )}
                    </div>
                  ))}
                </div>
              );
            })}
          </div>
          <div className="pane-foot small">
            <span className="muted">대기 2 · 실행 1</span>
            <button className="small-btn">일시정지</button>
          </div>
        </Panel>
        <Splitter dir="x" label="프로젝트 패널 너비" onDrag={(d) => setLeft((w) => clamp(w + d, 220, 480))} />

        {/* 프로그램 모니터 */}
        <Panel id="monitor" title={<>프로그램: <b>{job.title}</b></>} folded={false}
          className="area-monitor"
          tools={<><span className="faint small">미리보기 540×960</span></>}>
          <div className="monitor">
            <div className="frame" aria-label="미리보기 화면">
              {failed ? (
                <div className="frame-empty small">렌더에 실패해 미리보기가 없습니다.<br />로그에서 실패한 단계를 확인하세요.</div>
              ) : (
                <>
                  <div className="f-title">{job.title}</div>
                  <div className="f-media"><span>본문 사진 2 / 3</span></div>
                  <div className="f-cap">사진 속 스킨은 기계 갑옷에<br />금색 장식이 들어가 있습니다</div>
                  <div className="f-src">출처: 오버워치2 마이너 갤러리</div>
                </>
              )}
            </div>
          </div>
          <div className="transport">
            <span className="tc num">{fmt(head)}</span>
            <div className="scrub"><div className="scrub-fill" style={{width: `${(head / TOTAL) * 100}%`}} />
              {SCENES.slice(1).map((_, i) => {
                const at = SCENES.slice(0, i + 1).reduce((a, s) => a + s.secs, 0);
                return <span key={i} className="scrub-tick" style={{left: `${(at / TOTAL) * 100}%`}} />;
              })}
            </div>
            <span className="tc num faint">{fmt(TOTAL)}</span>
          </div>
          <div className="transport-btns">
            <button className="icon-btn" aria-label="이전 장면">|◁</button>
            <button className="icon-btn play-big" aria-label="재생"><Icon name="play" /></button>
            <button className="icon-btn" aria-label="다음 장면">▷|</button>
            <span className="grow" />
            <button disabled={failed}>편집기로 보내기</button>
            <button className="danger-btn">폐기</button>
            <button className="primary" disabled={failed || job.state !== 'review'}>승인하고 렌더</button>
          </div>
        </Panel>
        <Splitter dir="x" label="속성 패널 너비" onDrag={(d) => setRight((w) => clamp(w - d, 280, 520))} />

        {/* 속성 */}
        <Panel id="props" title={fold.props ? '속성' : <>속성 <span className="faint">· 장면 {scene + 1} {SCENES[scene].label}</span></>}
          folded={!!fold.props} onFold={() => toggle('props')} className="area-props">
          <Section title="대본" extra={<span className="faint small">A안 · 41초</span>}>
            {LINES.map((l, i) => (
              <div key={i} className={l.flag ? 'line flagged' : 'line'}>
                <span className="part">{l.part}</span>
                <input defaultValue={l.text} aria-label={`${i + 1}번 줄`} />
                <button className="play" aria-label="이 줄 듣기"><Icon name="play" size={12} /></button>
              </div>
            ))}
            {LINES.some((l) => l.flag) && <div className="warnline small">3번 줄: 원문과 다름</div>}
            <div className="actions tight">
              <button className="small-btn">B안 보기</button>
              <button className="small-btn">대본 다시 만들기</button>
            </div>
          </Section>
          <Section title="보이스">
            <div className="kv"><span className="muted">목소리</span><select defaultValue="a"><option value="a">도미니코 (복제)</option></select></div>
            <div className="kv"><span className="muted">속도</span><input type="range" min={80} max={130} defaultValue={108} aria-label="속도" /><span className="num small">1.08×</span></div>
          </Section>
          <Section title="스타일" extra={<span className="faint small">이슈형</span>}>
            <div className="kv"><span className="muted">스타일</span><select defaultValue="a" aria-label="스타일">
              <option value="a">이슈 기본</option><option>이슈 기본 사본</option><option>정보 카드</option></select></div>
            <div className="actions tight">
              <button className="small-btn" data-automation-id="style-copy">복사</button>
              <button className="small-btn" onClick={() => setLearn(true)} data-automation-id="style-learn">학습해서 보충·새로 만들기…</button>
            </div>
            <p className="faint small hint">자막 글꼴 Pretendard 64 · 강조 노랑 · 컷 템포 보통 · 줌 느리게</p>
          </Section>
          <Section title="출처 표시">
            <Switch checked={source} onChange={setSource} label="화면 아래에 출처 글자 넣기" />
          </Section>
          <Section title="원문" open={false} extra={<span className="faint small">댓글 48</span>}>
            <p className="small">원문 내용</p>
          </Section>
        </Panel>

        <Splitter dir="y" label="아래 패널 높이" onDrag={(d) => setBottom((h) => clamp(h - d, 160, 520))} />

        {/* 아래: 타임라인 / 로그 */}
        <section className={`pane area-bottom ${fold.bottom ? 'folded' : ''}`} data-automation-id="pane-bottom">
          <header className="pane-head tabs" role="tablist">
            <button className="fold" onClick={() => toggle('bottom')} aria-expanded={!fold.bottom} aria-label={fold.bottom ? '펼치기' : '접기'}>
              <Icon name="chevron" className={fold.bottom ? 'turn' : 'turn open'} /></button>
            <button role="tab" aria-selected={tab === 'timeline'} className={tab === 'timeline' ? 'tab on' : 'tab'}
              onClick={() => setTab('timeline')} data-automation-id="tab-timeline">타임라인</button>
            <button role="tab" aria-selected={tab === 'log'} className={tab === 'log' ? 'tab on' : 'tab'}
              onClick={() => setTab('log')} data-automation-id="tab-log">로그 <span className="error num">1</span></button>
            <span className="pane-tools">
              {tab === 'timeline' ? (
                <span className="faint small num">전체 {TOTAL.toFixed(1)}초 · 장면 {SCENES.length}개</span>
              ) : (
                <>
                  <select value={logJob} onChange={(e) => setLogJob(e.target.value as 'sel' | 'all')} aria-label="로그 범위" className="mini">
                    <option value="sel">선택한 글만</option><option value="all">모든 작업</option></select>
                  <Switch checked={onlyBad} onChange={setOnlyBad} label="오류·경고만" />
                  <Switch checked={follow} onChange={setFollow} label="실시간 따라가기" />
                  <button className="small-btn">복사</button>
                  <button className="small-btn">로그 폴더 열기</button>
                </>
              )}
            </span>
          </header>
          {!fold.bottom && tab === 'timeline' && (
            <div className="tl">
              <div className="tl-ruler">
                <span className="tl-name" />
                <div className="tl-track ruler">
                  {Array.from({length: Math.floor(TOTAL / 5) + 1}, (_, i) => (
                    <span key={i} className="tick num" style={{left: `${((i * 5) / TOTAL) * 100}%`}}>{i * 5}초</span>
                  ))}
                </div>
              </div>
              <div className="tl-row">
                <span className="tl-name">장면</span>
                <div className="tl-track">
                  {SCENES.map((s, i) => (
                    <button key={s.kind} className={i === scene ? 'clip on' : 'clip'} style={{flexGrow: s.secs}}
                      onClick={() => setScene(i)} data-automation-id={`scene-${s.kind}`}>
                      <span className="clip-name">{s.label}</span><span className="clip-len num">{s.secs.toFixed(1)}초</span>
                    </button>
                  ))}
                </div>
              </div>
              <div className="tl-row">
                <span className="tl-name">자막</span>
                <div className="tl-track subs">
                  {[3.2, 2.8, 3.1, 2.4, 3.0, 3.3, 2.6, 3.4, 2.9, 3.1, 2.7, 3.0, 2.2].map((w, i) => (
                    <span key={i} className="sub" style={{flexGrow: w}} />
                  ))}
                </div>
              </div>
              <div className="tl-row">
                <span className="tl-name">음성</span>
                <div className="tl-track wave" aria-hidden="true">
                  {Array.from({length: 120}, (_, i) => (
                    <span key={i} style={{height: `${18 + Math.abs(Math.sin(i * 0.7) * Math.cos(i * 0.23)) * 70}%`}} />
                  ))}
                </div>
              </div>
              <div className="tl-row">
                <span className="tl-name">배경음</span>
                <div className="tl-track"><span className="bgm">bgm_chill_02 · -18 dB</span></div>
              </div>
              <div className="playhead" style={{left: `calc(72px + (100% - 72px) * ${head / TOTAL})`}} />
            </div>
          )}
          {!fold.bottom && tab === 'log' && (
            <div className="logv">
              <ol className="stages" aria-label="단계">
                {STAGES.map((s) => {
                  const bad = LOGS.some((l) => l.job === sel && l.stage === s && l.level === 'error');
                  const done = LOGS.some((l) => l.job === sel && l.stage === s);
                  return (
                    <li key={s} className={bad ? 'bad' : done ? 'done' : ''}>
                      <span className="mark">{bad ? '실패' : done ? '완료' : ''}</span>{s}
                    </li>
                  );
                })}
              </ol>
              <div className="loglist" role="log">
                {logs.map((l, i) => (
                  <div key={i} className={`lrow ${l.level}`}>
                    <span className="faint num">{l.t}</span>
                    <span className={`lvl ${l.level}`}>{l.level === 'error' ? '오류' : l.level === 'warn' ? '경고' : '정보'}</span>
                    <span className="lstage">{l.stage}</span>
                    {logJob === 'all' && <span className="faint">#{l.job}</span>}
                    <span className="lmsg">{l.msg}</span>
                    {l.level === 'error' && <button className="small-btn">이 단계부터 다시</button>}
                  </div>
                ))}
                {follow && <div className="faint small live">실시간으로 따라가는 중</div>}
              </div>
            </div>
          )}
        </section>
      </div>

      <Dialog open={learn} title="스타일 학습" wide automationId="learn-dialog" onDismiss={() => setLearn(false)}>
        <p className="muted small">자료에서 자막·컷 템포·줌·색·효과음 자리 같은 특징을 모아 스타일에 넣습니다.</p>
        <div className="learn-src">
          <div className="lsrc">
            <div><b>Premiere 프로젝트 파일</b><div className="faint small">.prproj · 자막 스타일, 컷 길이, 전환, 글꼴</div></div>
            <button className="small-btn">파일 고르기</button>
          </div>
          <div className="lsrc">
            <div><b>통과된 쇼츠</b><div className="faint small">내가 승인한 쇼츠 중에서 고르기 · 2개 고름</div></div>
            <button className="small-btn" onClick={() => setPassed(true)}>목록 열기</button>
          </div>
          <div className="lsrc col">
            <div><b>유튜브 링크</b><div className="faint small">Claude Code 가 영상을 보고 특징을 모읍니다 (한 줄에 하나)</div></div>
            <textarea rows={3} defaultValue={'https://youtube.com/shorts/lKP7Ixw1ixc\nhttps://youtube.com/shorts/Fhkb3c2_xpc'} aria-label="유튜브 링크" />
          </div>
        </div>
        <div className="learn-to">
          <label className="check"><input type="radio" name="to" checked={learnTo === 'fill'} onChange={() => setLearnTo('fill')} />
            지금 스타일(이슈 기본)을 보충 <span className="faint small">바뀌는 항목을 먼저 보여 줍니다</span></label>
          <label className="check"><input type="radio" name="to" checked={learnTo === 'new'} onChange={() => setLearnTo('new')} />
            새 스타일 만들기</label>
        </div>
        <div className="actions">
          <button onClick={() => setLearn(false)}>취소</button>
          <button className="primary">학습 시작</button>
        </div>
      </Dialog>

      <Dialog open={passed} title="통과된 쇼츠에서 고르기" automationId="passed-dialog" onDismiss={() => setPassed(false)}>
        <input className="search" placeholder="제목 검색" aria-label="쇼츠 검색" />
        <div className="passed">
          {[
            ['시즌 20 밴율 정리', '정보 · 44초 · 11/20', true],
            ['아나 재우기 장인 등장', '옵갤 · 33초 · 11/20', true],
            ['겐지 용검 타이밍 모음', '게임 · 38초 · 11/18', false],
            ['이번 패치 너프 3줄 요약', '정보 · 29초 · 11/17', false],
            ['정커퀸 칼 회수 버그 제보', '이슈 · 41초 · 11/16', false],
          ].map(([t, s, on]) => (
            <label key={t as string} className="prow">
              <input type="checkbox" defaultChecked={on as boolean} />
              <span className="thumb" aria-hidden="true" />
              <span className="grow"><span>{t as string}</span><br /><span className="faint small">{s as string}</span></span>
            </label>
          ))}
        </div>
        <div className="actions">
          <button onClick={() => setPassed(false)}>취소</button>
          <button className="primary" onClick={() => setPassed(false)}>2개 넣기</button>
        </div>
      </Dialog>
    </div>
  );
}
