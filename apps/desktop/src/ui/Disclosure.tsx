import { useState, type PropsWithChildren, type ReactNode, type Ref } from 'react';
import { UiIcon } from './UiIcon';
import type { Language } from './preferences';

export function disclosureActionLabel(open: boolean, language: Language): string {
  if (language === 'zh') return open ? '收起' : '展开';
  return open ? 'Collapse' : 'Expand';
}

export function Disclosure({
  label,
  language,
  defaultOpen = false,
  open: controlledOpen,
  summaryRef,
  className = '',
  onOpenChange,
  children,
}: PropsWithChildren<{
  label: ReactNode;
  open?: boolean;
  summaryRef?: Ref<HTMLElement>;
  language: Language;
  defaultOpen?: boolean;
  className?: string;
  onOpenChange?(open: boolean): void;
}>) {
  const [localOpen, setOpen] = useState(defaultOpen);
  const open = controlledOpen ?? localOpen;
  return <details
    className={`disclosure ${className}`.trim()}
    open={open}
    data-state={open ? 'open' : 'closed'}
    onToggle={event => {
      setOpen(event.currentTarget.open);
      onOpenChange?.(event.currentTarget.open);
    }}
  >
    <summary aria-expanded={open} ref={summaryRef}>
      <span className="disclosure-label">{label}</span>
      <span className="disclosure-action">
        {disclosureActionLabel(open, language)}
        <UiIcon name="chevron" />
      </span>
    </summary>
    <div className="disclosure-body">{children}</div>
  </details>;
}
