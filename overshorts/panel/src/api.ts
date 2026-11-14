// 패널 ↔ FastAPI. 모든 요청은 실패해도 화면이 죽지 않게 오류 문장으로 돌려준다.
export async function api<T>(path: string, body?: unknown): Promise<T> {
  const res = await fetch(path, body === undefined ? {} : {
    method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify(body),
  });
  if (!res.ok) {
    let msg = `요청 실패 (${res.status})`;
    try {
      const j = await res.json();
      if (typeof j?.detail === 'string') msg = j.detail;
    } catch { /* 본문 없음 */ }
    throw new Error(msg);
  }
  return res.json() as Promise<T>;
}

export type Event = {type: string; time?: string; message?: string; stage?: string; label?: string;
  state?: string; level?: string; kind?: string};

/** WebSocket 구독. 끊기면 3초 뒤 다시 붙는다. */
export function subscribe(onEvent: (e: Event) => void): () => void {
  let sock: WebSocket | null = null;
  let closed = false;
  const open = () => {
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    sock = new WebSocket(`${proto}://${location.host}/ws`);
    sock.onmessage = (m) => {
      try { onEvent(JSON.parse(m.data)); } catch { /* 무시 */ }
    };
    sock.onclose = () => { if (!closed) setTimeout(open, 3000); };
  };
  open();
  return () => { closed = true; sock?.close(); };
}

export type Status = {
  today: {collected: number; generated: number; approved: number};
  review_waiting: number;
  budget: {hour_used: number; hour_limit: number; day_used: number; day_limit: number;
    cooldown_until: number | null; stopped_today: boolean};
  budget_warnings: string[];
  claude: {limited: boolean; message: string | null};
  gpu: {vendor: string; name: string};
  worker: {current: string | null; pending: number; paused: boolean};
};

export type Job = {id: number; status: string; error: string | null; updated_at: string;
  title: string; post_id: number; gall_id: string; post_no: number};

export type ScriptLine = {part: 'hook' | 'body' | 'comment' | 'outro' | 'pause'; text: string;
  comment_no?: number | null; seg?: number; min_total?: number};
export type Segment = {kind: 'title' | 'media' | 'body' | 'comments'; subtitle: string; tts: boolean;
  tts_reason: string; lines: string[]; media: {name: string; display: string; line: number}[];
  comments: number[]; read: number[]; quote?: string};
export type Comment = {no?: number; text: string; nick?: string; ip?: string; icon?: string; depth?: number;
  parent?: number; dccon?: {name: string} | null};
export type ReviewItem = {
  job_id: number; post_id: number; title: string; body: string; comments: Comment[]; preset: string;
  gall_name: string; post_url: string; overlap_ratio: number; tone: string; version: number;
  script: {title: string; fact: string; lines: ScriptLine[]; est_seconds: number;
    segments?: Segment[]; composition?: 'A' | 'B'; composition_reason?: string;
    cringe: {score: number; issues: {line: number; reason: string}[]} | null; warnings: string[]};
  filters: {stage: string; verdict: string; reasons: string[]; fit_score: number | null}[];
  spec: Record<string, unknown> | null; files: string;
};
