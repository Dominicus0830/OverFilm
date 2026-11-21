import React, {useEffect, useRef, useState} from 'react';
import {Icon} from './Icon';

/** 패널 사이 막대: 끌거나(마우스) 방향키로 옆 패널 크기를 바꾼다. */
export function Splitter({dir, onDrag, label}: {dir: 'x' | 'y'; onDrag: (d: number) => void; label: string}) {
  const last = useRef<number | null>(null);
  return (
    <div className={`split split-${dir}`} role="separator" aria-orientation={dir === 'x' ? 'vertical' : 'horizontal'}
      aria-label={label} tabIndex={0} data-automation-id={`split-${label}`}
      onKeyDown={(e) => {
        const k: Record<string, number> = dir === 'x' ? {ArrowLeft: -16, ArrowRight: 16} : {ArrowUp: -16, ArrowDown: 16};
        const d = k[e.key];
        if (d) { e.preventDefault(); onDrag(d); }
      }}
      onPointerDown={(e) => { last.current = dir === 'x' ? e.clientX : e.clientY; e.currentTarget.setPointerCapture(e.pointerId); }}
      onPointerMove={(e) => {
        if (last.current === null) return;
        const now = dir === 'x' ? e.clientX : e.clientY;
        onDrag(now - last.current); last.current = now;
      }}
      onPointerUp={() => { last.current = null; }} onPointerCancel={() => { last.current = null; }} />
  );
}

/** 패널 틀: 제목 줄(접기 · 이름 · 오른쪽 도구)과 내용. onFold 가 없으면 접을 수 없는 패널. */
export function Pane({id, title, tools, folded = false, onFold, children, className, bodyRef, right}: {
  id: string; title: React.ReactNode; tools?: React.ReactNode; folded?: boolean; onFold?: () => void;
  /** 오른쪽 가장자리 패널: 접힌 화살표가 안쪽(왼쪽)을 본다 */
  right?: boolean;
  children?: React.ReactNode; className?: string; bodyRef?: React.Ref<HTMLDivElement>;
}) {
  return (
    <section className={`pane ${className ?? ''} ${folded ? 'folded' : ''}`} data-automation-id={`pane-${id}`}
      aria-label={typeof title === 'string' ? title : undefined}>
      <header className="pane-head">
        {onFold ? (
          <button className="fold" onClick={onFold} aria-expanded={!folded} aria-label={folded ? '펼치기' : '접기'}
            data-automation-id={`pane-${id}-fold`}><span className={folded ? (right ? 'rot-left' : 'rot') : ''}><Icon name="chevron" /></span></button>
        ) : <span className="fold-gap" />}
        <span className="pane-title">{title}</span>
        {!folded && tools && <span className="pane-tools">{tools}</span>}
      </header>
      {/* 접어도 내용은 남겨 둔다(다른 패널이 그 안에 그리는 경우가 있어서). 보이지만 않게 */}
      <div className="pane-body" ref={bodyRef} hidden={folded}>{children}</div>
    </section>
  );
}

/** 속성 패널의 접히는 묶음. 열림 상태는 이름별로 기억한다. */
export function Section({id, title, defaultOpen = true, extra, children}: {
  id: string; title: string; defaultOpen?: boolean; extra?: React.ReactNode; children: React.ReactNode;
}) {
  const key = `section.${id}`;
  const [open, setOpen] = useState<boolean>(() => {
    try { const v = localStorage.getItem(key); return v === null ? defaultOpen : v === '1'; } catch { return defaultOpen; }
  });
  const toggle = () => {
    setOpen(!open);
    try { localStorage.setItem(key, open ? '0' : '1'); } catch { /* 기억 못 해도 됨 */ }
  };
  // 다른 곳에서 '이 묶음 열어 보여 줘' (openSection)
  useEffect(() => {
    const on = (e: Event) => {
      if ((e as CustomEvent<string>).detail !== id) return;
      setOpen(true);
      requestAnimationFrame(() => document.getElementById(`section-${id}`)?.scrollIntoView({behavior: 'smooth', block: 'start'}));
    };
    window.addEventListener('section:open', on);
    return () => window.removeEventListener('section:open', on);
  }, [id]);
  return (
    <div className="prop" data-automation-id={`section-${id}`} id={`section-${id}`}>
      <div className="prop-head">
        <button className="prop-toggle" onClick={toggle} aria-expanded={open} data-automation-id={`section-${id}-toggle`}>
          <span className={open ? '' : 'rot'}><Icon name="chevron" size={12} /></span>{title}
        </button>
        {extra && <span className="prop-extra">{extra}</span>}
      </div>
      {open && <div className="prop-body">{children}</div>}
    </div>
  );
}

/** 크기·접힘처럼 다음에 켤 때도 그대로였으면 하는 값(브라우저에만 저장, 못 하면 기본값). */
export function useStored<T>(key: string, initial: T): [T, (v: T | ((old: T) => T)) => void] {
  const [v, setV] = useState<T>(() => {
    try { const s = localStorage.getItem(key); return s === null ? initial : (JSON.parse(s) as T); } catch { return initial; }
  });
  const set = (next: T | ((old: T) => T)) => setV((old) => {
    const val = typeof next === 'function' ? (next as (o: T) => T)(old) : next;
    try { localStorage.setItem(key, JSON.stringify(val)); } catch { /* 기억 못 해도 됨 */ }
    return val;
  });
  return [v, set];
}

/** 속성 패널의 묶음 하나를 열고 그리로 스크롤한다. */
export function openSection(id: string) {
  window.dispatchEvent(new CustomEvent('section:open', {detail: id}));
}
