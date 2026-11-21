import React, {useId} from 'react';
import * as S from '@radix-ui/react-switch';

/** 켬/끔 스위치(4px 사각). 글자는 늘 붙이고, 상태는 모양과 aria-checked 로 알린다. */
export function Switch({checked, onChange, label, hint, automationId, disabled}: {
  checked: boolean; onChange: (v: boolean) => void; label: string; hint?: string; automationId?: string;
  disabled?: boolean;
}) {
  const id = useId();
  return (
    <span className="switch" title={hint}>
      <S.Root id={id} className="switch-root" checked={checked} onCheckedChange={onChange} disabled={disabled}
        data-automation-id={automationId}>
        <S.Thumb className="switch-thumb" />
      </S.Root>
      <label htmlFor={id}>{label}</label>
    </span>
  );
}
