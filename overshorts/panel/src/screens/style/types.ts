/** 스타일(편집 유형) 화면 공용 타입. 화면 배치가 바뀌어도 컴포넌트를 그대로 옮길 수 있게 따로 둔다. */
export type {PresetInfo} from '../EditOptions';
export type PassedShort = {id: number; status: string; updated_at: string; title: string};
/** 학습·대화 결과: 아직 저장 전인 프리셋과 요약. */
export type Draft = {preset: Record<string, unknown>; summary: string[]};
export type LearnAnswer = {ok: boolean; error?: string; reply?: string; findings?: string[];
  preset?: Record<string, unknown>; summary?: string[]; base?: string | null;
  used?: {premiere: boolean; shorts: number[]; links: string[]}};
