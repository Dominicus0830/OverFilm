import React, {useEffect, useState} from 'react';
import {api} from '../api';
import {StyleChat, type Turn} from '../screens/style/StyleChat';
import {StyleLearn} from '../screens/style/StyleLearn';
import {StyleList} from '../screens/style/StyleList';
import type {Draft, LearnAnswer, PresetInfo} from '../screens/style/types';
import {Section, openSection} from '../ui/Pane';

/**
 * 속성 패널의 스타일 묶음: 예전 '스타일' 화면(목록·복사·보충 / 학습 / Claude Code 대화)을
 * 접히는 묶음 셋으로 옮긴 것. 컴포넌트는 screens/style/ 의 것을 그대로 쓴다.
 */
export function StylePanel({tick}: {tick: number}) {
  const [list, setList] = useState<PresetInfo[]>([]);
  const [turns, setTurns] = useState<Turn[]>([]);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [base, setBase] = useState('');
  const [msg, setMsg] = useState('');

  const load = () => api<PresetInfo[]>('/api/presets').then(setList).catch((e) => setMsg(e.message));
  useEffect(() => { load(); }, [tick]);

  const supplement = (id: string) => {
    setBase(id);
    openSection('style-learn');
  };
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
    <>
      <Section id="style" title="스타일 (편집 유형)" extra={<span className="faint small num">{list.length}개</span>}>
        <StyleList compact list={list} onChanged={load} onSupplement={supplement} onMessage={setMsg} />
        {msg && <p className={/^(저장|복사)했습니다/.test(msg) ? 'note ok' : 'error'} role="status">{msg}</p>}
      </Section>
      <Section id="style-learn" title="스타일 학습" defaultOpen={false}>
        <StyleLearn list={list} base={base} setBase={setBase} onResult={learned} />
      </Section>
      <Section id="style-chat" title="내 유형 만들기 (Claude Code 대화)" defaultOpen={false}>
        <StyleChat turns={turns} setTurns={setTurns} draft={draft} setDraft={setDraft}
          onSaved={(m) => { setMsg(m); load(); }} />
      </Section>
    </>
  );
}
