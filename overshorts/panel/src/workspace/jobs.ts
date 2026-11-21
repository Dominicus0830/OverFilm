import type {Event, Job} from '../api';

/** 작업 상태 → 한국어. */
export const STATUS_KO: Record<string, string> = {
  queued: '대기', crawled: '수집됨', filtered: '필터 통과', scripted: '대본', voiced: '음성', aligned: '자막 타이밍',
  preview_ready: '검수 대기', approved: '승인 (렌더 중)', rendered: '완료', blocked: '막힘', rejected: '폐기', failed: '실패',
};

/** 파이프라인 단계(로그 패널 왼쪽 목록): 어느 상태까지 오면 그 단계가 끝난 것인지. */
export const STAGES: {key: string; label: string; done: string[]}[] = [
  {key: 'crawl', label: '수집', done: ['crawled', 'filtered', 'scripted', 'voiced', 'aligned', 'preview_ready', 'approved', 'rendered']},
  {key: 'filter', label: '필터', done: ['filtered', 'scripted', 'voiced', 'aligned', 'preview_ready', 'approved', 'rendered']},
  {key: 'script', label: '대본', done: ['scripted', 'voiced', 'aligned', 'preview_ready', 'approved', 'rendered']},
  {key: 'tts', label: '음성', done: ['voiced', 'aligned', 'preview_ready', 'approved', 'rendered']},
  {key: 'align', label: '자막 타이밍', done: ['aligned', 'preview_ready', 'approved', 'rendered']},
  {key: 'preview', label: '미리보기·검수', done: ['approved', 'rendered']},
  {key: 'render', label: '최종 렌더', done: ['rendered']},
];

/** 단계 목록에서 지금 막힌(실패한) 단계: 끝나지 않은 첫 단계. */
export function failedStage(j: Job): string | null {
  if (!j.error && j.status !== 'blocked' && j.status !== 'failed') return null;
  if (j.status === 'blocked') return 'filter';
  return STAGES.find((s) => !s.done.includes(j.status))?.key ?? null;
}

export type Group = 'review' | 'run' | 'wait' | 'fail' | 'done' | 'rejected';
export const GROUPS: {key: Group; label: string}[] = [
  {key: 'review', label: '검수 대기'}, {key: 'run', label: '만드는 중'}, {key: 'wait', label: '대기'},
  {key: 'fail', label: '실패·막힘'}, {key: 'done', label: '완료'}, {key: 'rejected', label: '폐기'},
];

export function groupOf(j: Job): Group {
  if (j.status === 'rejected') return 'rejected';
  if (j.error || j.status === 'blocked' || j.status === 'failed') return 'fail';
  if (j.status === 'preview_ready') return 'review';
  if (j.status === 'rendered') return 'done';
  if (j.status === 'queued') return 'wait';
  return 'run';
}

export function statusText(j: Job): string {
  if (j.status === 'approved' && j.error) return '렌더 실패';
  if (j.error && j.status !== 'blocked') return `${STATUS_KO[j.status] ?? j.status} 다음 단계 실패`;
  return STATUS_KO[j.status] ?? j.status;
}

export const jobName = (j: Job) => j.title || `${j.gall_id} ${j.post_no}`;

/** 이 이벤트가 이 글 것인가: 작업 이름표('작업 412 …')나 글 번호·제목이 들어 있으면. 모르면 아니라고 본다. */
export function eventOf(e: Event, j: Job | undefined): boolean {
  if (!j) return false;
  const text = `${e.label ?? ''} ${e.message ?? ''}`;
  return text.includes(`작업 ${j.id}`) || text.includes(String(j.post_no)) || (!!j.title && text.includes(j.title));
}

/** 오류·경고 이벤트인가. */
export function isBad(e: Event): boolean {
  return e.level === 'warn' || e.level === 'error' || e.type === 'alert'
    || (e.type === 'task' && (e.state === 'failed' || e.state === 'cancelled'));
}
