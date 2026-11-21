import React from 'react';

/* OverFilm 선형 아이콘: 편집기(crates/ui-egui/src/icons.rs)와 OverFilmSubs 가 쓰는 같은 그림을
   SVG 로 옮긴 것. 16×16 격자, 선 굵기 1.25, 글자색(currentColor) 한 가지, 외부 아이콘·이모지 없음.
   직접 그린 그림이라 라이선스는 저장소와 같다(docs/design.md 의 아이콘 항목). */

type Pt = [number, number];

const line = (pts: Pt[]) => pts.map(([x, y], i) => `${i ? 'L' : 'M'}${+x.toFixed(2)} ${+y.toFixed(2)}`).join(' ');
const arcPts = (cx: number, cy: number, r: number, a0: number, a1: number, n = 20): Pt[] =>
  Array.from({length: n + 1}, (_, i) => {
    const a = ((a0 + ((a1 - a0) * i) / n) * Math.PI) / 180;
    return [cx + r * Math.cos(a), cy + r * Math.sin(a)];
  });
const rays = (r0: number, r1: number): string =>
  Array.from({length: 8}, (_, i) => {
    const a = (i * 45 * Math.PI) / 180;
    return line([[8 + r0 * Math.cos(a), 8 + r0 * Math.sin(a)], [8 + r1 * Math.cos(a), 8 + r1 * Math.sin(a)]]);
  }).join(' ');

/** 선으로 그리는 부분(stroke)과 채우는 부분(fill). */
const SHAPES: Record<string, {stroke?: string; fill?: string; circles?: [number, number, number][]; dots?: [number, number, number][]}> = {
  home: {stroke: [line([[1.5, 8], [8, 2], [14.5, 8]]), line([[3.5, 6.5], [3.5, 14], [12.5, 14], [12.5, 6.5]]),
    line([[6.5, 14], [6.5, 10], [9.5, 10], [9.5, 14]])].join(' ')},
  plus: {stroke: [line([[8, 3], [8, 13]]), line([[3, 8], [13, 8]])].join(' ')},
  list: {stroke: [4, 8, 12].map((y) => line([[5.5, y], [14, y]])).join(' '), dots: [[3, 4, 0.8], [3, 8, 0.8], [3, 12, 0.8]]},
  eye: {stroke: line([[1.5, 8], [4, 5], [8, 3.8], [12, 5], [14.5, 8], [12, 11], [8, 12.2], [4, 11], [1.5, 8]]), circles: [[8, 8, 2]]},
  mic: {stroke: [line([[6, 2], [10, 2], [10, 10], [6, 10], [6, 2]]), line(arcPts(8, 8, 4.5, 0, 180)), line([[8, 12.5], [8, 14.5]])].join(' ')},
  adjust: {stroke: [4, 8, 12].map((y) => line([[2, y], [14, y]])).join(' '), dots: [[10, 4, 1.5], [5, 8, 1.5], [9, 12, 1.5]]},
  folder: {stroke: line([[1.5, 4], [6, 4], [7.5, 5.5], [14.5, 5.5], [14.5, 13], [1.5, 13], [1.5, 4]])},
  gear: {stroke: rays(4, 6), circles: [[8, 8, 2.2], [8, 8, 4.2]]},
  sun: {stroke: rays(4.6, 6.2), circles: [[8, 8, 2.8]]},
  moon: {stroke: line([...arcPts(8, 8.5, 5.5, 8.56, 261.44, 12), ...arcPts(11.2, 5.3, 4.6, 209.12, 60.88, 12).slice(1)]) + ' Z'},
  play: {fill: 'M4.5 2.8 L13 8 L4.5 13.2 Z'},
  stop: {fill: 'M4 4 H12 V12 H4 Z'},
  record: {dots: [[8, 8, 4]]},
  chevron: {stroke: line([[4, 6], [8, 10], [12, 6]])},
};

export type IconName = keyof typeof SHAPES;

/** 글자 옆에 놓는 아이콘. 뜻은 옆 글자나 aria-label 이 전하므로 스크린리더에서는 숨긴다. */
export function Icon({name, size = 16, className}: {name: IconName; size?: number; className?: string}) {
  const s = SHAPES[name];
  return (
    <svg className={className ? `icon ${className}` : 'icon'} width={size} height={size} viewBox="0 0 16 16"
      aria-hidden="true" focusable="false">
      {s.stroke && <path d={s.stroke} fill="none" stroke="currentColor" strokeWidth={1.25} strokeLinecap="round" strokeLinejoin="round" />}
      {s.circles?.map(([cx, cy, r]) => <circle key={`c${cx}${cy}${r}`} cx={cx} cy={cy} r={r} fill="none" stroke="currentColor" strokeWidth={1.25} />)}
      {s.dots?.map(([cx, cy, r]) => <circle key={`d${cx}${cy}${r}`} cx={cx} cy={cy} r={r} fill="currentColor" />)}
      {s.fill && <path d={s.fill} fill="currentColor" />}
    </svg>
  );
}
