import React, {useCallback, useEffect, useMemo, useRef, useState} from 'react';
import {Player, PlayerRef} from '@remotion/player';
import {CardStory} from '../../../render/src/CardStory';
import {sanitize} from '../../../render/src/spec';
import {api, ReviewItem, ScriptLine} from '../api';
import type {Screen} from '../App';

const PART_KO: Record<string, string> = {hook: '훅', body: '전개', comment: '댓글 반응', outro: '마무리'};
const norm = (s: string) => s.replace(/[\s\p{P}\p{S}]+/gu, '');

/** 원문에서 대본과 겹치는 글자(글자 5-gram)를 표시한다. */
function overlapMarks(body: string, lines: ScriptLine[]): boolean[] {
  const script = norm(lines.filter((l) => l.part !== 'comment').map((l) => l.text).join(''));
  const grams = new Set<string>();
  for (let i = 0; i + 5 <= script.length; i++) grams.add(script.slice(i, i + 5));
  const idx: number[] = []; // 정규화 글자 → 원문 위치
  const chars: string[] = [];
  [...body].forEach((ch, i) => { if (norm(ch)) { idx.push(i); chars.push(ch); } });
  const marks = new Array<boolean>([...body].length).fill(false);
  const flat = chars.join('');
  for (let i = 0; i + 5 <= chars.length; i++) {
    if (grams.has(flat.slice(i, i + 5))) for (let k = i; k < i + 5; k++) marks[idx[k]] = true;
  }
  return marks;
}

export function Review({tick, go}: {tick: number; go: (s: Screen) => void}) {
  const [items, setItems] = useState<ReviewItem[]>([]);
  const [pos, setPos] = useState(0);
  const [lines, setLines] = useState<ScriptLine[]>([]);
  const [dirty, setDirty] = useState(false);
  const [regen, setRegen] = useState(false);
  const [report, setReport] = useState<{line: number; reason: string} | null>(null);
  const [msg, setMsg] = useState('');
  const player = useRef<PlayerRef>(null);

  useEffect(() => { api<ReviewItem[]>('/api/review').then(setItems).catch((e) => setMsg(e.message)); }, [tick]);
  const item = items[Math.min(pos, Math.max(0, items.length - 1))];
  useEffect(() => { setLines(item?.script.lines ?? []); setDirty(false); setRegen(false); setReport(null); },
    [item?.job_id, item?.version]);

  const spec = useMemo(() => {
    if (!item?.spec) return null;
    const s = sanitize(item.spec);
    return {...s, audio: s.audio ? item.files + s.audio : null,
      background: s.background ? item.files + s.background : null};
  }, [item]);

  const act = useCallback(async (what: 'approve' | 'edit' | 'discard') => {
    if (!item) return;
    setMsg('');
    try {
      if (what === 'edit') await api(`/api/review/${item.job_id}/edit`, {lines});
      else await api(`/api/review/${item.job_id}/${what}`, {});
      setMsg(what === 'approve' ? '승인: 최종 렌더를 시작했습니다' : what === 'edit' ? '수정 반영: 음성부터 다시 만듭니다' : '폐기했습니다');
      setItems((old) => old.filter((x) => x.job_id !== item.job_id));
    } catch (e) { setMsg((e as Error).message); }
  }, [item, lines]);

  const regenerate = async (body: {tone?: string; preset?: string}) => {
    if (!item) return;
    try {
      await api(`/api/review/${item.job_id}/regenerate`, body);
      setMsg(body.tone ? `톤 '${body.tone}'(으)로 대본부터 다시 씁니다` : '렌더 설정만 다시 만듭니다');
      setItems((old) => old.filter((x) => x.job_id !== item.job_id));
    } catch (e) { setMsg((e as Error).message); }
    setRegen(false);
  };

  // 단축키: J/K 다음·이전, A 승인, E 수정 반영, R 재생성, D 폐기 (입력 중에는 끔)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || e.ctrlKey || e.metaKey || e.altKey) return;
      const k = e.key.toLowerCase();
      if (k === 'j') setPos((p) => Math.min(p + 1, Math.max(0, items.length - 1)));
      else if (k === 'k') setPos((p) => Math.max(0, p - 1));
      else if (k === 'a') act('approve');
      else if (k === 'e' && dirty) act('edit');
      else if (k === 'r') setRegen((r) => !r);
      else if (k === 'd') act('discard');
      else return;
      e.preventDefault();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [items.length, act, dirty]);

  const playLine = (i: number) => {
    const sc = spec?.scenes.filter((s) => s.line === i);
    if (!spec || !sc?.length || !player.current) return;
    const from = Math.round(sc[0].start * spec.fps);
    const to = Math.round(sc[sc.length - 1].end * spec.fps);
    player.current.seekTo(from);
    player.current.play();
    const stop = () => {
      if ((player.current?.getCurrentFrame() ?? to) >= to) { player.current?.pause(); return; }
      requestAnimationFrame(stop);
    };
    requestAnimationFrame(stop);
  };

  const sendReport = async () => {
    if (!item || !report) return;
    await api(`/api/review/${item.job_id}/cringe`, {line: report.line, text: lines[report.line]?.text ?? '', reason: report.reason});
    setMsg('오글 신고를 저장했습니다. 스타일 화면에서 금지 목록 반영을 승인할 수 있습니다.');
    setReport(null);
  };

  if (!item) {
    return (
      <section className="empty">
        <h1>검수</h1>
        <p>검수할 영상이 없습니다.</p>
        <button className="primary" onClick={() => go('새로 만들기')}>링크 넣기</button>
        {msg && <p className="muted">{msg}</p>}
      </section>
    );
  }
  const flagged = new Map((item.script.cringe?.issues ?? []).map((i) => [i.line, i.reason]));
  const marks = overlapMarks(item.body, lines);
  const bodyChars = [...item.body];
  const llm = item.filters.filter((f) => f.stage === 'llm').slice(-1)[0];

  return (
    <section className="review">
      <header className="review-head">
        <h1>검수 대기 목록 <span className="muted small">{pos + 1} / {items.length} · J/K</span></h1>
        {msg && <span className="badge">{msg}</span>}
      </header>
      <div className="review-cols">
        <div className="col">
          <h3>원문</h3>
          <p className="muted small">제목·본문·고른 댓글 · <a href={item.post_url} target="_blank" rel="noreferrer">원문 열기</a></p>
          <h4>{item.title}</h4>
          <p className="body">{bodyChars.map((ch, i) => marks[i] ? <mark key={i}>{ch}</mark> : ch)}</p>
          <p className="muted small">노란색 = 대본과 겹치는 부분</p>
          <ul className="comments">
            {item.comments.slice(0, 50).map((c, i) => {
              const picked = lines.some((l) => l.part === 'comment' && l.comment_no === i);
              return <li key={i} className={picked ? 'picked' : ''}>[{i}] {c.text}</li>;
            })}
          </ul>
        </div>
        <div className="col">
          <h3>대본 (문장별 편집)</h3>
          <p className="muted small">▶ 누르면 그 문장만 듣기 · 예상 {item.script.est_seconds}초</p>
          {lines.map((l, i) => (
            <div key={i} className={flagged.has(i) ? 'line flagged' : 'line'}>
              <button className="play" aria-label={`${i + 1}번째 문장 듣기`} onClick={() => playLine(i)}>▶</button>
              <span className="part">{PART_KO[l.part] ?? l.part}</span>
              <input value={l.text} aria-label={`${i + 1}번째 문장`} onChange={(e) => {
                const next = lines.slice(); next[i] = {...l, text: e.target.value}; setLines(next); setDirty(true);
              }} />
              {flagged.has(i) && <span className="badge yellow" title={flagged.get(i)}>오글?</span>}
              <button className="small-btn" onClick={() => setReport({line: i, reason: ''})}>오글</button>
            </div>
          ))}
          {report && (
            <div className="card">
              <p>{report.line + 1}번째 문장이 오글거리는 이유 (선택)</p>
              <input autoFocus value={report.reason} onChange={(e) => setReport({...report, reason: e.target.value})}
                onKeyDown={(e) => { if (e.key === 'Enter') sendReport(); }} />
              <div className="actions"><button onClick={sendReport}>저장</button><button onClick={() => setReport(null)}>취소</button></div>
            </div>
          )}
          <p className="muted small">톤: {item.tone} · 프리셋: 썰 카드형 · 대본 {item.version}번째</p>
        </div>
        <div className="col">
          <h3>미리보기</h3>
          {spec ? (
            <Player ref={player} component={CardStory} inputProps={spec as unknown as Record<string, unknown>}
              durationInFrames={Math.max(1, Math.round(spec.duration * spec.fps))} fps={spec.fps}
              compositionWidth={spec.width} compositionHeight={spec.height} controls
              style={{width: '100%', aspectRatio: '9 / 16', borderRadius: 16, overflow: 'hidden'}} />
          ) : <p className="muted">미리보기가 없습니다</p>}
          <p className="muted small">9:16 Player, 렌더 없이 재생</p>
        </div>
      </div>
      <footer className="review-foot">
        <div className="muted small">
          필터: {llm ? `${llm.verdict}${llm.fit_score !== null ? ` · 적합도 ${llm.fit_score}` : ''}` : '규칙만'}
          {' · '}원문 겹침 {Math.round(item.overlap_ratio * 100)}%{item.overlap_ratio > 0.3 ? ' (30% 초과)' : ''}
          {' · '}오글 점수 {item.script.cringe ? item.script.cringe.score : '-'}
          {item.script.warnings.length > 0 && <div className="warnline">경고: {item.script.warnings.join(' / ')}</div>}
        </div>
        <div className="actions">
          <button className="primary" onClick={() => act('approve')} data-automation-id="review-approve">승인 (A)</button>
          <button disabled={!dirty} onClick={() => act('edit')} data-automation-id="review-edit">수정 반영 (E)</button>
          <div className="menu">
            <button onClick={() => setRegen(!regen)} data-automation-id="review-regen">재생성 ▾ (R)</button>
            {regen && (
              <div className="menu-pop">
                <p className="muted small">톤 바꾸기 → 대본부터</p>
                {['담백', '드립 조금', '정리형'].map((t) => <button key={t} onClick={() => regenerate({tone: t})}>{t}</button>)}
                <p className="muted small">프리셋만 → 렌더 설정만</p>
                <button onClick={() => regenerate({preset: 'card'})}>썰 카드형</button>
              </div>
            )}
          </div>
          <button onClick={() => act('discard')} data-automation-id="review-discard">폐기 (D)</button>
        </div>
      </footer>
    </section>
  );
}
