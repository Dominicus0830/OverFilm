import React, {useEffect, useRef, useState} from 'react';
import {api} from '../api';
import {VoicesInfo, playVoice, voiceName} from './Voices';

// 검수 화면: 이 영상만 다른 목소리로 읽기 (바꾸면 음성부터 다시 만든다).
export function VoicePicker({jobId}: {jobId: number}) {
  const [info, setInfo] = useState<VoicesInfo | null>(null);
  const [cur, setCur] = useState<{override: {narrator?: string; comment?: string};
    effective: {narrator: string; comment: string | null}} | null>(null);
  const [open, setOpen] = useState(false);
  const [narrator, setNarrator] = useState('');
  const [comment, setComment] = useState('');
  const [msg, setMsg] = useState('');
  const audio = useRef<HTMLAudioElement | null>(null);

  useEffect(() => {
    setMsg('');
    api<VoicesInfo>('/api/voices').then(setInfo).catch(() => setInfo(null));
    api<typeof cur>(`/api/voices/job/${jobId}`).then((c) => {
      setCur(c); setNarrator(c?.override.narrator ?? ''); setComment(c?.override.comment ?? '');
    }).catch(() => setCur(null));
  }, [jobId]);
  if (!info || !cur) return null;

  const apply = async () => {
    try {
      await api(`/api/voices/job/${jobId}`, {narrator: narrator || null, comment: comment || null});
      setMsg('목소리를 바꿨습니다: 음성부터 다시 만듭니다'); setOpen(false);
    } catch (e) { setMsg(String((e as Error).message ?? e)); }
  };
  const listen = async () => {
    const id = narrator || cur.effective.narrator;
    setMsg('합성하는 중…');
    try { await playVoice(id, '', audio); setMsg(''); } catch (e) { setMsg(String((e as Error).message ?? e)); }
  };
  const options = info.voices.map((v) => <option key={v.id} value={v.id}>{v.name}{v.kind === 'clone' ? ' (클로닝)' : ''}</option>);
  return (
    <div className="muted small" data-automation-id="review-voice">
      목소리: {voiceName(info, cur.effective.narrator)}
      {cur.effective.comment ? ` · 댓글 ${voiceName(info, cur.effective.comment)}` : ''}
      {cur.override.narrator || cur.override.comment ? ' (이 영상만)' : ''}
      {' '}<button className="small-btn" onClick={() => setOpen(!open)} data-automation-id="review-voice-change">바꾸기</button>
      {open && (
        <div className="card" style={{marginTop: 6}}>
          <label className="field"><span>나레이션</span>
            <select value={narrator} onChange={(e) => setNarrator(e.target.value)}>
              <option value="">보이스 화면 기본값</option>{options}</select></label>
          <label className="field"><span>댓글</span>
            <select value={comment} onChange={(e) => setComment(e.target.value)}>
              <option value="">보이스 화면 기본값</option>
              <option value="same">나레이션과 같은 목소리</option>{options}</select></label>
          <div className="actions">
            <button className="primary" onClick={apply} data-automation-id="review-voice-apply">적용 (음성 다시)</button>
            <button onClick={listen}>나레이션 미리듣기</button>
            <button onClick={() => setOpen(false)}>취소</button>
          </div>
        </div>
      )}
      {msg && <div>{msg}</div>}
    </div>
  );
}
