import React, {useEffect, useState} from 'react';
import {api} from '../api';
import {StyleChat, type Turn} from './style/StyleChat';
import {StyleLearn} from './style/StyleLearn';
import {StyleList} from './style/StyleList';
import type {Draft, LearnAnswer, PresetInfo} from './style/types';

/**
 * 스타일(편집 유형) 화면: 목록(복사·보충·지우기) + 학습 + Claude Code 대화.
 * 각 부분은 `screens/style/` 의 독립 컴포넌트라 패널 배치가 바뀌어도 따로 옮길 수 있다.
 */
export function StyleMaker() {
  const [list, setList] = useState<PresetInfo[]>([]);
  const [turns, setTurns] = useState<Turn[]>([]);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [base, setBase] = useState('');
  const [msg, setMsg] = useState('');

  const load = () => api<PresetInfo[]>('/api/presets').then(setList).catch((e) => setMsg(e.message));
  useEffect(() => { load(); }, []);

  const supplement = (id: string) => {
    setBase(id);
    document.querySelector('[data-automation-id="style-learn"]')?.scrollIntoView({behavior: 'smooth'});
  };
  // 학습 결과를 대화에 이어 붙인다: 대화로 더 고치거나 바로 저장할 수 있다.
  const learned = (a: LearnAnswer) => {
    if (!a.preset) return;
    const used = [a.used?.premiere ? 'Premiere 파일' : '', a.used?.shorts.length ? `통과된 쇼츠 ${a.used.shorts.length}개` : '',
      a.used?.links.length ? `유튜브 ${a.used.links.length}개` : ''].filter(Boolean).join(', ');
    const target = a.base ? `'${list.find((p) => p.id === a.base)?.label ?? a.base}' 보충` : '새 스타일';
    const found = (a.findings ?? []).map((f) => `· ${f}`).join('\n');
    setTurns([...turns, {role: 'user', text: `학습 (${target}): ${used}`},
      {role: 'assistant', text: [a.reply ?? '', found].filter(Boolean).join('\n')}]);
    setDraft({preset: a.preset, summary: a.summary ?? []});
    setMsg('');
  };

  return (
    <section>
      <h1>스타일 (편집 유형)</h1>
      <StyleList list={list} onChanged={load} onSupplement={supplement} onMessage={setMsg} />
      {msg && <p className={/^(저장|복사)했습니다/.test(msg) ? 'note ok' : 'error'} role="status">{msg}</p>}
      <StyleLearn list={list} base={base} setBase={setBase} onResult={learned} />
      <h2>내 유형 만들기</h2>
      <StyleChat turns={turns} setTurns={setTurns} draft={draft} setDraft={setDraft}
        onSaved={(m) => { setMsg(m); load(); }} />
    </section>
  );
}
