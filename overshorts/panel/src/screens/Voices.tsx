import React, {useEffect, useRef, useState} from 'react';
import {api} from '../api';

// 보이스 화면 (기획서 3.7): 기성 보이스 미리듣기, 클로닝 등록 마법사(녹음/업로드 → 대사 → 동의 → 테스트 합성),
// 나레이션·댓글 목소리 고르기. 클로닝은 GPT-SoVITS 로컬 서버, 실패하면 Edge-TTS 로 읽는다.

export type VoiceItem = {id: string; name: string; kind: 'preset' | 'clone'; voice_id?: number; ref_text?: string;
  ref_sec?: number; sample_sec?: number; created_at?: string};
export type VoicesInfo = {
  voices: VoiceItem[]; selection: {narrator?: string | null; comment?: string | null};
  effective: {narrator: string; comment: string | null};
  config: {narrator?: string; comment?: string}; fallback: {narrator: string; comment: string};
  read_sentences: string[]; consent_text: string; ref_sec: [number, number];
  clone: {server: string; dir: string | null; alive: boolean; managed: boolean; cpu_warning: string | null;
    align: 'stable_ts' | 'syllable'};
};

/** POST /api/voices/preview → 오디오를 받아 바로 재생. 실패하면 오류 문장. */
export async function playVoice(voice: string, text: string, audio: React.MutableRefObject<HTMLAudioElement | null>) {
  const res = await fetch('/api/voices/preview', {method: 'POST', headers: {'Content-Type': 'application/json'},
    body: JSON.stringify({voice, text})});
  if (!res.ok) {
    let msg = `미리듣기 실패 (${res.status})`;
    try { const j = await res.json(); if (typeof j?.detail === 'string') msg = j.detail; } catch { /* 본문 없음 */ }
    throw new Error(msg);
  }
  const url = URL.createObjectURL(await res.blob());
  audio.current?.pause();
  const a = new Audio(url);
  audio.current = a;
  a.onended = () => URL.revokeObjectURL(url);
  await a.play();
}

export function voiceName(info: VoicesInfo | null, id: string | null | undefined): string {
  if (!id) return '없음';
  if (id === 'same') return '나레이션과 같은 목소리';
  return info?.voices.find((v) => v.id === id)?.name ?? id;
}

export function Voices({tick}: {tick: number}) {
  const [info, setInfo] = useState<VoicesInfo | null>(null);
  const [err, setErr] = useState('');
  const [msg, setMsg] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  const [text, setText] = useState('');
  const audio = useRef<HTMLAudioElement | null>(null);
  const load = () => api<VoicesInfo>('/api/voices').then((i) => { setInfo(i); setErr(''); })
    .catch((e) => setErr(String(e.message ?? e)));
  useEffect(() => { load(); }, [tick]);

  if (err) return <p className="error">보이스 정보를 읽지 못했습니다: {err}</p>;
  if (!info) return <p className="muted">불러오는 중…</p>;

  const preview = async (id: string) => {
    setBusy(id); setMsg(id.startsWith('clone:') && !info.clone.alive
      ? '클로닝 서버를 켜고 합성하는 중… (처음엔 1~3분 걸릴 수 있음)' : '합성하는 중…');
    try { await playVoice(id, text, audio); setMsg(''); } catch (e) { setMsg(String((e as Error).message ?? e)); }
    setBusy(null); load();
  };
  const select = async (key: 'narrator' | 'comment', value: string) => {
    try {
      await api('/api/voices/selection', {[key]: value || null});
      setMsg(`${key === 'narrator' ? '나레이션' : '댓글'} 목소리를 바꿨습니다. 다음에 만드는 영상부터 적용됩니다`);
      load();
    } catch (e) { setMsg(String((e as Error).message ?? e)); }
  };
  const server = async (action: 'start' | 'stop') => {
    try {
      await api('/api/voices/server', {action});
      setMsg(action === 'start' ? '서버를 켜는 중입니다. 모델을 올리는 데 1~3분 걸릴 수 있습니다' : '서버를 껐습니다');
      setTimeout(load, 1500);
    } catch (e) { setMsg(String((e as Error).message ?? e)); }
  };
  const remove = async (v: VoiceItem) => {
    if (!v.voice_id || !window.confirm(`'${v.name}' 목소리를 지울까요? 샘플 파일도 같이 지워집니다.`)) return;
    try {
      await fetch(`/api/voices/${v.voice_id}`, {method: 'DELETE'});
      load();
    } catch (e) { setMsg(String((e as Error).message ?? e)); }
  };
  const rename = async (v: VoiceItem) => {
    const name = window.prompt('새 이름', v.name);
    if (!name || !v.voice_id) return;
    try { await api(`/api/voices/${v.voice_id}/rename`, {name}); load(); }
    catch (e) { setMsg(String((e as Error).message ?? e)); }
  };
  const playRef = (v: VoiceItem) => {
    audio.current?.pause();
    audio.current = new Audio(`/api/voices/${v.voice_id}/ref`);
    audio.current.play().catch(() => setMsg('참조 음성을 재생하지 못했습니다'));
  };

  const clones = info.voices.filter((v) => v.kind === 'clone');
  const presets = info.voices.filter((v) => v.kind === 'preset');
  const sel = info.selection;
  return (
    <section>
      <h1>보이스</h1>
      {msg && <p className="badge" role="status">{msg}</p>}
      <div className="cards">
        <div className="card">
          <h3>지금 쓰는 목소리</h3>
          <label className="field"><span>나레이션</span>
            <select value={sel.narrator ?? ''} onChange={(e) => select('narrator', e.target.value)}
              data-automation-id="voice-narrator">
              <option value="">설정값 ({voiceName(info, info.config.narrator)})</option>
              {info.voices.map((v) => <option key={v.id} value={v.id}>{v.kind === 'clone' ? '🎙 ' : ''}{v.name}</option>)}
            </select></label>
          <label className="field"><span>댓글 읽는 목소리</span>
            <select value={sel.comment ?? ''} onChange={(e) => select('comment', e.target.value)}
              data-automation-id="voice-comment">
              <option value="">설정값 ({voiceName(info, info.config.comment)})</option>
              <option value="same">나레이션과 같은 목소리 (화자 분리 끔)</option>
              {info.voices.map((v) => <option key={v.id} value={v.id}>{v.kind === 'clone' ? '🎙 ' : ''}{v.name}</option>)}
            </select></label>
          <p className="muted small">클로닝이 실패하면 그 영상은 기성 보이스({voiceName(info, info.fallback.narrator)}
            {' · '}{voiceName(info, info.fallback.comment)})로 읽고 검수 화면에 경고를 남깁니다.
            검수 화면에서 영상마다 목소리를 따로 바꿀 수도 있습니다.</p>
        </div>
        <div className="card">
          <h3>클로닝 서버 (GPT-SoVITS)</h3>
          <p className={info.clone.alive ? 'badge ok' : 'badge'}>{info.clone.alive ? '켜짐' : '꺼짐'}
            {info.clone.managed ? ' · OverShorts 가 띄움' : ''}</p>
          <p className="muted small">주소 {info.clone.server}<br />
            {info.clone.dir ? `폴더 ${info.clone.dir}` : '폴더를 정하지 않았습니다: 직접 띄우거나 config.yaml 의 voice.clone.dir 에 GPT-SoVITS 폴더를 적으면 필요할 때 자동으로 켭니다'}</p>
          {info.clone.cpu_warning && <p className="badge warn">{info.clone.cpu_warning}</p>}
          <p className="muted small">단어 자막 시각: {info.clone.align === 'stable_ts' ? 'stable-ts 강제 정렬'
            : '음절 수 비율 (pip install stable-ts 하면 더 정확)'}</p>
          <div className="actions">
            <button onClick={() => server('start')} disabled={info.clone.alive || !info.clone.dir}
              data-automation-id="voice-server-start">서버 켜기</button>
            <button onClick={() => server('stop')} disabled={!info.clone.managed}
              data-automation-id="voice-server-stop">서버 끄기</button>
            <button onClick={load}>새로 고침</button>
          </div>
          <p className="muted small">음성 단계가 끝나면 OverShorts 가 띄운 서버는 자동으로 내립니다 (메모리 확보).</p>
        </div>
      </div>

      <div className="card">
        <h3>목소리 목록</h3>
        <label className="field"><span>미리듣기 문장 (비우면 기본 문장)</span>
          <input value={text} maxLength={200} onChange={(e) => setText(e.target.value)}
            placeholder="오늘은 오버워치 갤러리에서 화제가 된 글을 가져왔어." data-automation-id="voice-preview-text" /></label>
        <table className="jobs">
          <tbody>
            {[...presets, ...clones].map((v) => (
              <tr key={v.id}>
                <td>{v.kind === 'clone' ? '🎙 ' : ''}<b>{v.name}</b>
                  <div className="muted small">{v.kind === 'clone'
                    ? `클로닝 · 참조 ${v.ref_sec ?? '?'}초 · “${v.ref_text ?? ''}”` : `기성 보이스 (Edge-TTS) · ${v.id.slice(5)}`}</div></td>
                <td style={{whiteSpace: 'nowrap', textAlign: 'right'}}>
                  <button className="small-btn" disabled={busy !== null} onClick={() => preview(v.id)}
                    data-automation-id={`voice-preview-${v.id}`}>{busy === v.id ? '합성 중…' : '▶ 미리듣기'}</button>
                  {v.kind === 'clone' && <>
                    {' '}<button className="small-btn" onClick={() => playRef(v)}>참조 음성</button>
                    {' '}<button className="small-btn" onClick={() => rename(v)}>이름</button>
                    {' '}<button className="small-btn" onClick={() => remove(v)}
                      data-automation-id={`voice-delete-${v.voice_id}`}>삭제</button></>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {clones.length === 0 && <p className="muted small">아직 등록한 클로닝 목소리가 없습니다.</p>}
      </div>

      <Register info={info} onDone={(name) => { setMsg(`'${name}' 목소리를 등록했습니다`); load(); }} audio={audio} />
    </section>
  );
}

type Picked = {blob: Blob; name: string; duration: number | null; speech: [number, number] | null};

/** 브라우저에서 길이와 앞뒤 무음을 잰다 (실패하면 서버가 다시 잰다). */
async function analyze(blob: Blob): Promise<{duration: number; speech: [number, number]} | null> {
  try {
    const Ctx = window.AudioContext || (window as unknown as {webkitAudioContext: typeof AudioContext}).webkitAudioContext;
    const ctx = new Ctx();
    const buf = await ctx.decodeAudioData(await blob.arrayBuffer());
    ctx.close().catch(() => undefined);
    const d = buf.getChannelData(0);
    const step = Math.max(1, Math.round(buf.sampleRate * 0.02));
    const levels: number[] = [];
    for (let i = 0; i + step <= d.length; i += step) {
      let s = 0;
      for (let j = i; j < i + step; j++) s += d[j] * d[j];
      levels.push(Math.sqrt(s / step));
    }
    const peak = Math.max(...levels, 0);
    const floor = Math.max(0.002, peak * 0.06);
    const first = levels.findIndex((l) => l >= floor);
    let last = levels.length - 1;
    while (last > 0 && levels[last] < floor) last--;
    const s = first < 0 ? 0 : Math.max(0, first * 0.02 - 0.15);
    const e = first < 0 ? buf.duration : Math.min(buf.duration, (last + 1) * 0.02 + 0.15);
    return {duration: buf.duration, speech: [Math.round(s * 100) / 100, Math.round(e * 100) / 100]};
  } catch {
    return null;
  }
}

function Register({info, onDone, audio}: {info: VoicesInfo; onDone: (name: string) => void;
  audio: React.MutableRefObject<HTMLAudioElement | null>}) {
  const [picked, setPicked] = useState<Picked | null>(null);
  const [recording, setRecording] = useState(false);
  const rec = useRef<MediaRecorder | null>(null);
  const [sentence, setSentence] = useState(0);
  const [refText, setRefText] = useState(info.read_sentences[0] ?? '');
  const [range, setRange] = useState<[string, string]>(['', '']);
  const [name, setName] = useState('');
  const [consent, setConsent] = useState(false);
  const [advanced, setAdvanced] = useState(false);
  const [weights, setWeights] = useState({gpt: '', sovits: ''});
  const [state, setState] = useState('');
  const [busy, setBusy] = useState(false);
  const [min, max] = info.ref_sec;

  const take = async (blob: Blob, fileName: string) => {
    const a = await analyze(blob);
    setPicked({blob, name: fileName, duration: a?.duration ?? null, speech: a?.speech ?? null});
    if (a) {
      const [s, e] = a.speech;
      // 말소리가 10초를 넘으면 앞에서 10초만 골라 두고 대사를 고치게 한다
      setRange([String(s), String(Math.min(e, Math.round((s + max - 0.2) * 100) / 100))]);
    } else setRange(['', '']);
    setState('');
  };
  const startRec = async () => {
    try {
      const stream = await navigator.mediaDevices.getUserMedia({audio: true});
      const chunks: Blob[] = [];
      const r = new MediaRecorder(stream);
      r.ondataavailable = (e) => { if (e.data.size) chunks.push(e.data); };
      r.onstop = () => {
        stream.getTracks().forEach((t) => t.stop());
        take(new Blob(chunks, {type: r.mimeType || 'audio/webm'}), '녹음');
      };
      rec.current = r; r.start(); setRecording(true);
    } catch {
      setState('마이크를 쓸 수 없습니다. 브라우저 권한을 확인하거나 파일을 올려 주세요');
    }
  };
  const stopRec = () => { rec.current?.stop(); setRecording(false); };
  const pickSentence = (i: number) => { setSentence(i); setRefText(info.read_sentences[i] ?? ''); };
  const len = Number(range[1]) - Number(range[0]);
  const lenOk = !picked?.speech || (len >= min && len <= max);

  const submit = async () => {
    if (!picked) return;
    setBusy(true); setState('등록하는 중…');
    try {
      const data = await new Promise<string>((ok, bad) => {
        const r = new FileReader(); r.onload = () => ok(String(r.result)); r.onerror = bad; r.readAsDataURL(picked.blob);
      });
      const body: Record<string, unknown> = {name: name.trim(), data, ref_text: refText.trim(), consent,
        gpt_weights: weights.gpt.trim(), sovits_weights: weights.sovits.trim()};
      if (range[0] !== '' && range[1] !== '') { body.start = Number(range[0]); body.end = Number(range[1]); }
      const res = await api<{voice: VoiceItem}>('/api/voices/register', body);
      setState('등록했습니다. 테스트 합성 중… (서버가 꺼져 있으면 켜는 데 1~3분)');
      try { await playVoice(res.voice.id, '', audio); setState('테스트 합성을 재생합니다. 마음에 안 들면 지우고 다시 녹음하세요'); }
      catch (e) { setState(`등록은 됐지만 테스트 합성에 실패했습니다: ${(e as Error).message}`); }
      onDone(res.voice.name);
      setPicked(null); setConsent(false); setName('');
    } catch (e) {
      setState(String((e as Error).message ?? e));
    }
    setBusy(false);
  };

  return (
    <div className="card" style={{marginTop: 14}}>
      <h3>클로닝 목소리 등록</h3>
      <p className="muted small">본인 목소리이거나 목소리 주인에게 동의받은 목소리만 등록하세요.
        조용한 곳에서 {min}~{max}초 동안 또박또박 말한 녹음이 가장 잘 됩니다.</p>

      <h4>1. 녹음하거나 파일 올리기</h4>
      <p>아래 문장 중 하나를 그대로 읽어 주세요:</p>
      {info.read_sentences.map((s, i) => (
        <label key={i} className="check" style={{justifyContent: 'flex-start', margin: '4px 0'}}>
          <input type="radio" style={{width: 'auto', flex: 'none'}} checked={sentence === i}
            onChange={() => pickSentence(i)} /><span>“{s}”</span></label>
      ))}
      <div className="actions">
        {!recording
          ? <button className="primary" onClick={startRec} data-automation-id="voice-record">● 녹음 시작</button>
          : <button className="primary" onClick={stopRec} data-automation-id="voice-record-stop">■ 녹음 끝</button>}
        <label className="small-btn" style={{cursor: 'pointer'}}>파일 올리기 (wav·mp3·m4a, 15MB 까지)
          <input type="file" accept="audio/*,video/webm" style={{display: 'none'}} data-automation-id="voice-upload"
            onChange={(e) => { const f = e.target.files?.[0]; if (f) take(f, f.name); e.target.value = ''; }} /></label>
        {picked && <button onClick={() => {
          audio.current?.pause(); audio.current = new Audio(URL.createObjectURL(picked.blob)); audio.current.play();
        }}>▶ 들어 보기</button>}
      </div>
      {picked && <p className="muted small">{picked.name}{picked.duration !== null ? ` · ${picked.duration.toFixed(1)}초` : ''}
        {picked.speech ? ` · 말소리 ${picked.speech[0]}~${picked.speech[1]}초` : ''}</p>}

      {picked && <>
        <h4>2. 참조 구간과 대사</h4>
        <p className="muted small">GPT-SoVITS 는 {min}~{max}초 참조 음성만 받습니다. 구간을 정하고, 그 구간에서 한 말을 글자 그대로 적어 주세요.</p>
        <div className="actions">
          <label>시작(초) <input type="number" step="0.1" min={0} value={range[0]} style={{width: 90}}
            onChange={(e) => setRange([e.target.value, range[1]])} /></label>
          <label>끝(초) <input type="number" step="0.1" min={0} value={range[1]} style={{width: 90}}
            onChange={(e) => setRange([range[0], e.target.value])} /></label>
          {picked.speech && <span className={lenOk ? 'muted small' : 'error small'}>{len.toFixed(1)}초{lenOk ? '' : ` (${min}~${max}초로)`}</span>}
          {picked.speech && <button className="small-btn" onClick={() => {
            const a = new Audio(URL.createObjectURL(picked.blob)); audio.current?.pause(); audio.current = a;
            a.currentTime = Number(range[0]) || 0; a.play();
            const end = Number(range[1]);
            a.ontimeupdate = () => { if (end && a.currentTime >= end) a.pause(); };
          }}>▶ 구간만 듣기</button>}
        </div>
        <label className="field"><span>구간에서 한 말</span>
          <textarea rows={2} value={refText} maxLength={200} onChange={(e) => setRefText(e.target.value)}
            style={{width: '100%', boxSizing: 'border-box', font: 'inherit', padding: 8, borderRadius: 10}}
            data-automation-id="voice-ref-text" /></label>

        <h4>3. 이름과 동의</h4>
        <label className="field"><span>목소리 이름</span>
          <input value={name} maxLength={40} onChange={(e) => setName(e.target.value)} placeholder="예: 내 목소리"
            data-automation-id="voice-name" /></label>
        <label className="check" style={{justifyContent: 'flex-start'}}><input type="checkbox" style={{width: 'auto', flex: 'none'}} checked={consent} onChange={(e) => setConsent(e.target.checked)}
          data-automation-id="voice-consent" /> {info.consent_text}</label>
        <button className="small-btn" style={{marginTop: 8}} onClick={() => setAdvanced(!advanced)}>
          고급 (GPT-SoVITS 로 학습한 모델) {advanced ? '▴' : '▾'}</button>
        {advanced && <>
          <label className="field"><span>GPT 모델 (.ckpt, 선택)</span>
            <input value={weights.gpt} onChange={(e) => setWeights({...weights, gpt: e.target.value})}
              placeholder="GPT_weights_v2ProPlus/내목소리-e15.ckpt" /></label>
          <label className="field"><span>SoVITS 모델 (.pth, 선택)</span>
            <input value={weights.sovits} onChange={(e) => setWeights({...weights, sovits: e.target.value})}
              placeholder="SoVITS_weights_v2ProPlus/내목소리_e8_s200.pth" /></label>
          <p className="muted small">비우면 기본 모델로 참조 음성만 보고 흉내 냅니다(제로샷). 1분 이상 녹음으로 GPT-SoVITS 웹UI 에서 학습한 모델을 적으면 더 비슷해집니다.</p>
        </>}

        <h4>4. 등록하고 테스트 합성</h4>
        <div className="actions">
          <button className="primary" disabled={busy || !consent || !name.trim() || !refText.trim() || !lenOk}
            onClick={submit} data-automation-id="voice-register">등록하고 들어 보기</button>
        </div>
      </>}
      {state && <p className="badge" role="status">{state}</p>}
    </div>
  );
}
