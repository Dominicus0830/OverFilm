import React, {useEffect, useMemo, useState} from 'react';
import {api} from '../api';
import './settings.css';

// 설정 화면 (M8). 값은 config.yaml 에 저장되고 다시 켜지 않아도 다음 작업부터 쓴다.
// 보이스(목소리) 설정은 '보이스' 화면에서 한다.

type Kind = 'choice' | 'text' | 'path' | 'int' | 'float' | 'bool' | 'pair' | 'words';
type Field = {kind: Kind; choices: string[]; lo: number; hi: number; restart: boolean};
type Value = string | number | boolean | null | number[] | string[];
type Values = Record<string, Record<string, Value>>;
type Prefs = {skip_crawl_confirm: boolean; on_limit: 'fallback' | 'wait' | null};
type View = {values: Values; fields: Record<string, Field>; prefs: Prefs; path: string | null;
  data_dir: string; restart?: string[]};

type Item = {key: string; label: string; hint?: string; names?: Record<string, string>;
  unit?: string; step?: number; placeholder?: string};
type Group = {title: string; note?: string; items: Item[]};

const LLM = {ollama: '로컬 (Ollama)', claude_code: 'Claude Code (구독)'};

const GROUPS: Group[] = [
  {title: 'AI 모델', note: 'Ollama 는 이 컴퓨터 GPU 로 돌고(라데온 지원), Claude Code 는 구독 사용량을 씁니다.', items: [
    {key: 'llm.filter', label: '글 거르기', hint: '정치·혐오·선정성 같은 글을 거르는 모델', names: LLM},
    {key: 'llm.script', label: '대본 쓰기', hint: '대본·제목·오글 검사', names: LLM},
    {key: 'llm.fallback', label: '안 될 때 대신 쓸 모델', hint: 'Claude 한도나 오류가 나면', names: LLM},
    {key: 'llm.on_limit', label: 'Claude 한도에 걸리면', names: {ask: '물어보기', fallback: '로컬 모델로 넘기기', wait: '풀릴 때까지 기다리기'}},
    {key: 'llm.ollama_model', label: 'Ollama 모델', hint: '그래픽 메모리 6~7GB면 qwen3:4b', placeholder: 'qwen3:8b'},
    {key: 'llm.ollama_host', label: 'Ollama 주소', placeholder: 'http://127.0.0.1:11434'},
    {key: 'llm.claude_model', label: 'Claude 모델', hint: '비우면 Claude Code 기본 모델', placeholder: '기본 모델'},
  ]},
  {title: '대본·제목', items: [
    {key: 'style.speech', label: '말투', hint: '채널에 하나만'},
    {key: 'style.title_tail', label: '제목 꼬리표', hint: '제목 끝에 붙는 채널 말투',
      names: {auto: '글 분위기에 맞춰 (0~1개)', none: '안 붙임', 'ㄷㄷ': 'ㄷㄷ', 'ㅋㅋㅋ': 'ㅋㅋㅋ'}},
    {key: 'render.tone', label: '기본 톤', hint: '새로 만들기에서 바꾸지 않으면 이 톤'},
    {key: 'style.tts', label: '읽어 주기(TTS) 기본값', hint: '검수 화면에서 장면마다 다시 켜고 끌 수 있음',
      names: {auto: 'AI 가 장면마다 정함', on: '본문·댓글 항상 읽기', off: '자막만 (읽지 않음)'}},
    {key: 'style.min_cringe_score', label: '오글 검사 기준', hint: '0~10. 이보다 낮으면 대본을 다시 씀', step: 0.5},
  ]},
  {title: '영상', items: [
    {key: 'render.text_direction', label: '글 연출 AI 판단', hint: '장면마다 글 뜨고 사라지는 때, 크기·색·굵기·글꼴을 AI 가 정함. 끄면 규칙대로'},
    {key: 'render.show_source', label: '출처 표시', hint: "영상 아래 '출처: DC인사이드 …'. 이미 만든 미리보기도 바뀜"},
    {key: 'render.preset', label: '화면 모양', names: {auto: '자동 (원문 사진·영상 있으면 프레임형)', frame: '프레임형', card: '썰 카드형'}},
    {key: 'render.composition', label: '구성', names: {auto: 'AI 가 고름', A: 'A: 제목 → 본문 → 댓글', B: 'B: 제목 → 본문'}},
    {key: 'render.nickname', label: '닉네임', names: {original: '원본 그대로', blur: '흐리게', replace: 'ㅇㅇ 으로 바꿈'}},
    {key: 'render.max_comments', label: '댓글 최대', hint: '답글 포함', unit: '개'},
    {key: 'render.max_media', label: '원문 사진·영상 최대', unit: '개'},
    {key: 'general.encoder', label: '인코더', names: {auto: '자동', nvenc: 'NVIDIA (NVENC)', amf: 'AMD 라데온 (AMF)', cpu: 'CPU'}},
  ]},
  {title: '글 거르기', items: [
    {key: 'filter.min_chars', label: '최소 글자 수', hint: '제목+본문', unit: '자'},
    {key: 'filter.max_chars', label: '최대 글자 수', unit: '자'},
    {key: 'filter.blur_faces', label: '첨부 사진 얼굴 흐리기'},
    {key: 'filter.drop_nsfw_images', label: '선정적인 사진 빼기'},
    {key: 'filter.block_words', label: '금지어', hint: '한 줄에 하나. 이 낱말이 든 글은 받지 않음'},
  ]},
  {title: '수집 예산·요청 한도', note: '디시 차단을 피하려고 천천히 받습니다. 한도를 넘으면 다음 시간·다음 날까지 멈춥니다.', items: [
    {key: 'crawler.hourly_limit', label: '시간당 요청', unit: '회'},
    {key: 'crawler.daily_limit', label: '하루 요청', unit: '회'},
    {key: 'crawler.delay_sec', label: '요청 간격', hint: '이 사이에서 무작위. 1초 미만은 차단 위험', unit: '초', step: 0.5},
    {key: 'crawler.rest_every', label: '긴 휴식 주기', hint: '글 몇 개마다 (0 = 안 쉼)', unit: '개'},
    {key: 'crawler.rest_sec', label: '긴 휴식 시간', unit: '초'},
    {key: 'crawler.cooldown_min', label: '차단 신호 뒤 쉬기', hint: '403·429·캡차', unit: '분'},
    {key: 'crawler.max_comments', label: '글당 받을 댓글', unit: '개'},
    {key: 'crawler.max_images', label: '글당 받을 첨부 사진', unit: '장'},
    {key: 'crawler.proxy', label: '프록시', hint: '비우면 안 씀', placeholder: '없음'},
  ]},
  {title: '폴더', note: '상대 경로는 데이터 폴더(OverShorts 설정 파일이 있는 곳) 기준입니다.', items: [
    {key: 'render.out_dir', label: '다 만든 영상', hint: '비우면 데이터 폴더/approved', placeholder: 'data/approved'},
    {key: 'render.background_dir', label: '배경 영상', hint: '직접 녹화했거나 라이선스 있는 것만'},
    {key: 'render.bgm_dir', label: 'BGM', hint: '비우면 안 깜. 라이선스 있는 것만', placeholder: '없음'},
    {key: 'style.dir', label: '스타일 (말투 예시·금지 표현·짤)'},
    {key: 'general.data_dir', label: '작업·DB 폴더', hint: '바꾸면 OverShorts 를 다시 켜야 적용'},
    {key: 'render.browser_executable', label: '렌더용 Chrome', hint: '비우면 Remotion 이 받은 Chrome', placeholder: '자동'},
  ]},
];

function get(values: Values, key: string): Value {
  const [sec, k] = key.split('.');
  return values[sec]?.[k] ?? null;
}

function same(a: Value, b: Value): boolean {
  return JSON.stringify(a ?? null) === JSON.stringify(b ?? null);
}

export function Settings() {
  const [view, setView] = useState<View | null>(null);
  const [draft, setDraft] = useState<Values>({});
  const [msg, setMsg] = useState('');
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);

  const load = (v: View) => { setView(v); setDraft(JSON.parse(JSON.stringify(v.values))); };
  useEffect(() => { api<View>('/api/settings').then(load).catch((e) => setErr((e as Error).message)); }, []);

  const changed = useMemo(() => {
    if (!view) return [] as string[];
    return Object.keys(view.fields).filter((k) => !same(get(draft, k), get(view.values, k)));
  }, [view, draft]);

  const set = (key: string, v: Value) => {
    const [sec, k] = key.split('.');
    setDraft((d) => ({...d, [sec]: {...(d[sec] ?? {}), [k]: v}}));
    setMsg('');
  };

  const save = async (prefs?: Partial<Prefs>) => {
    setBusy(true); setErr(''); setMsg('');
    const values: Values = {};
    for (const key of changed) {
      const [sec, k] = key.split('.');
      (values[sec] ??= {})[k] = get(draft, key);
    }
    try {
      const r = await api<View & {restart: string[]}>('/api/settings', {values, prefs: prefs ?? {}});
      load(r);
      setMsg(r.restart.length ? '저장했습니다. 폴더 변경은 OverShorts 를 다시 켜면 적용됩니다.'
        : '저장했습니다. 다음 작업부터 적용됩니다.');
    } catch (e) {
      setErr((e as Error).message);
    } finally { setBusy(false); }
  };

  if (!view) {
    return <section><h1>설정</h1>{err ? <p className="error">{err}</p> : <p className="muted">불러오는 중…</p>}</section>;
  }

  return (
    <section className="settings">
      <h1>설정</h1>
      {GROUPS.map((g) => (
        <div key={g.title} className="set-group">
          <h3>{g.title}</h3>
          {g.note && <p className="muted small set-note">{g.note}</p>}
          <div className="set-list">
            {g.items.filter((it) => view.fields[it.key]).map((it) => (
              <Row key={it.key} item={it} field={view.fields[it.key]} value={get(draft, it.key)}
                dirty={changed.includes(it.key)} set={(v) => set(it.key, v)} />
            ))}
          </div>
        </div>
      ))}

      <div className="set-group">
        <h3>확인창</h3>
        <div className="set-list">
          <div className="set-row">
            <div className="set-label"><span>갤러리 수집 전에 예상 요청 수 보여 주기</span>
              <span className="muted small">새로 만들기에서 '다시 묻지 않기'를 골랐으면 꺼져 있습니다</span></div>
            <div className="set-control">
              {view.prefs.skip_crawl_confirm
                ? <button disabled={busy} onClick={() => save({skip_crawl_confirm: false})} data-automation-id="set-pref-crawl-confirm">다시 보이기</button>
                : <span className="muted small">보임</span>}
            </div>
          </div>
          <div className="set-row">
            <div className="set-label"><span>Claude 한도 확인창</span>
              <span className="muted small">{view.prefs.on_limit
                ? `확인창에서 '다시 묻지 않기'로 정함: ${view.prefs.on_limit === 'fallback' ? '로컬 모델로 넘기기' : '기다리기'}`
                : "위 'Claude 한도에 걸리면' 설정을 따릅니다"}</span></div>
            <div className="set-control">
              {view.prefs.on_limit && <button disabled={busy} onClick={() => save({on_limit: null})} data-automation-id="set-pref-on-limit">되돌리기</button>}
            </div>
          </div>
        </div>
      </div>

      <div className="set-foot">
        <div className="muted small">
          설정 파일: <code>{view.path ?? '없음 (저장할 수 없음)'}</code><br />
          데이터 폴더: <code>{view.data_dir}</code>
        </div>
        <div className="actions">
          {msg && <span className="state-ok" role="status">{msg}</span>}
          {err && <span className="error">{err}</span>}
          <button disabled={busy || !changed.length} onClick={() => { setDraft(JSON.parse(JSON.stringify(view.values))); setErr(''); }}
            data-automation-id="set-revert">되돌리기</button>
          <button className="primary" disabled={busy || !changed.length || !view.path} onClick={() => save()}
            data-automation-id="set-save">{changed.length ? `저장 (${changed.length})` : '저장'}</button>
        </div>
      </div>
    </section>
  );
}

function Row({item, field, value, dirty, set}: {item: Item; field: Field; value: Value; dirty: boolean;
  set: (v: Value) => void}) {
  const id = `set-${item.key}`;
  return (
    <div className={dirty ? 'set-row dirty' : 'set-row'}>
      <label className="set-label" htmlFor={id}>
        <span>{item.label}{field.restart && <span className="muted small"> (다시 켜야 적용)</span>}</span>
        {item.hint && <span className="muted small">{item.hint}</span>}
      </label>
      <div className="set-control">
        <Control id={id} item={item} field={field} value={value} set={set} />
      </div>
    </div>
  );
}

function Control({id, item, field, value, set}: {id: string; item: Item; field: Field; value: Value;
  set: (v: Value) => void}) {
  const auto = {id, 'data-automation-id': id};
  switch (field.kind) {
    case 'choice':
      return (
        <select {...auto} value={String(value ?? '')} onChange={(e) => set(e.target.value)}>
          {field.choices.map((c) => <option key={c} value={c}>{item.names?.[c] ?? c}</option>)}
        </select>
      );
    case 'bool':
      return <input {...auto} type="checkbox" className="set-switch" checked={value === true}
        onChange={(e) => set(e.target.checked)} />;
    case 'int':
    case 'float':
      return (
        <span className="set-num">
          <NumberBox {...auto} value={typeof value === 'number' ? value : field.lo} field={field}
            step={item.step ?? (field.kind === 'int' ? 1 : 0.1)} set={set} />
          {item.unit && <span className="muted">{item.unit}</span>}
        </span>
      );
    case 'pair': {
      const pair = Array.isArray(value) && value.length === 2 ? (value as number[]) : [field.lo, field.lo];
      return (
        <span className="set-num">
          <NumberBox id={id} data-automation-id={`${id}-min`} value={pair[0]} field={field} step={item.step ?? 1}
            set={(v) => set([Number(v), pair[1]])} />
          <span className="muted">~</span>
          <NumberBox id={`${id}-max`} data-automation-id={`${id}-max`} value={pair[1]} field={field} step={item.step ?? 1}
            set={(v) => set([pair[0], Number(v)])} />
          {item.unit && <span className="muted">{item.unit}</span>}
        </span>
      );
    }
    case 'words': {
      const words = Array.isArray(value) ? (value as string[]) : [];
      return <WordsBox {...auto} words={words} set={set} />;
    }
    default:
      return <input {...auto} value={String(value ?? '')} placeholder={item.placeholder}
        spellCheck={false} onChange={(e) => set(e.target.value)} />;
  }
}

/** 입력 중 빈칸·'1.' 같은 값도 받고, 범위 안 숫자만 위로 올린다. */
function NumberBox({value, field, step, set, ...rest}: {id: string; 'data-automation-id': string; value: number;
  field: Field; step: number; set: (v: Value) => void}) {
  const [text, setText] = useState(String(value));
  useEffect(() => { if (Number(text) !== value) setText(String(value)); }, [value]); // eslint-disable-line react-hooks/exhaustive-deps
  const n = Number(text);
  const bad = text.trim() === '' || !Number.isFinite(n) || n < field.lo || n > field.hi
    || (field.kind === 'int' && !Number.isInteger(n));
  return (
    <input {...rest} type="number" inputMode="decimal" min={field.lo} max={field.hi} step={step} value={text}
      className={bad ? 'bad' : undefined} title={bad ? `${field.lo}~${field.hi}` : undefined}
      onChange={(e) => { setText(e.target.value); const v = Number(e.target.value);
        if (e.target.value.trim() !== '' && Number.isFinite(v)) set(v); }} />
  );
}

/** 금지어: 한 줄에 하나. 쓰는 동안은 글 그대로 두고 줄 단위 목록으로 올린다. */
function WordsBox({words, set, ...rest}: {id: string; 'data-automation-id': string; words: string[];
  set: (v: Value) => void}) {
  const [text, setText] = useState(words.join('\n'));
  useEffect(() => {
    const cur = text.split('\n').map((w) => w.trim()).filter(Boolean);
    if (JSON.stringify(cur) !== JSON.stringify(words)) setText(words.join('\n'));
  }, [words]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <textarea {...rest} rows={Math.min(8, Math.max(3, words.length + 1))} value={text} spellCheck={false}
      onChange={(e) => { setText(e.target.value); set(e.target.value.split('\n').map((w) => w.trim()).filter(Boolean)); }} />
  );
}
