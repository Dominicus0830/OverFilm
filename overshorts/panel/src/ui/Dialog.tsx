import React from 'react';
import * as D from '@radix-ui/react-dialog';

/**
 * 확인창 하나. Radix Dialog 라서 포커스가 창 안에 갇히고, 닫히면 원래 버튼으로 돌아간다.
 * `onDismiss` 가 없으면 Esc·바깥 클릭으로 닫히지 않는다(답을 꼭 받아야 하는 창).
 */
export function Dialog({open, title, onDismiss, children, automationId, wide}: {
  open: boolean; title: string; onDismiss?: () => void; children: React.ReactNode; automationId?: string;
  /** 고를 것이 많은 창(스타일 학습 등)은 넓게 */
  wide?: boolean;
}) {
  const block = (e: Event) => { if (!onDismiss) e.preventDefault(); };
  return (
    <D.Root open={open} onOpenChange={(o) => { if (!o) onDismiss?.(); }}>
      <D.Portal>
        <D.Overlay className="modal-back" />
        <D.Content className={wide ? 'modal wide' : 'modal'} data-automation-id={automationId} aria-describedby={undefined}
          onEscapeKeyDown={block} onPointerDownOutside={block} onInteractOutside={block}>
          <D.Title className="modal-title">{title}</D.Title>
          {children}
        </D.Content>
      </D.Portal>
    </D.Root>
  );
}
