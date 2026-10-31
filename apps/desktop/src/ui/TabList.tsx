import { useEffect, useRef, useState, type ReactNode } from 'react';

type TabItem<T extends string> = { value: T; label: ReactNode; id?: string; controls?: string };

/** Arrow keys move focus. Guarded editors activate only on click/Enter/Space. */
export function TabList<const T extends string>({ items, value, onChange, label, className = '', activation = 'manual', focusScope }: {
  items: readonly TabItem<T>[];
  value: T;
  onChange(value: T): void | Promise<void>;
  label: string;
  className?: string;
  activation?: 'manual' | 'automatic';
  /** Peer page frames can mount separate copies of the same visible tab group. */
  focusScope?: string;
}) {
  const root = useRef<HTMLDivElement>(null);
  const [focused, setFocused] = useState(value);
  useEffect(() => setFocused(value), [value]);
  async function activate(next: T, keyboard: boolean) {
    await onChange(next);
    if (!keyboard) return;
    requestAnimationFrame(() => {
      const group = root.current?.getClientRects().length ? root.current : focusScope
        ? [...document.querySelectorAll<HTMLDivElement>('[data-tab-focus-scope]')]
          .find(node => node.dataset.tabFocusScope === focusScope && node.getClientRects().length) : null;
      group?.querySelector<HTMLButtonElement>('[role="tab"][aria-selected="true"]')?.focus();
    });
  }
  return <div ref={root} className={`tab-list ${className}`.trim()} role="tablist" aria-label={label} data-tab-focus-scope={focusScope}
    onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget)) setFocused(value); }}>
    {items.map((item, index) => <button key={item.value} id={item.id} type="button" role="tab"
      className={`segment${value === item.value ? ' active' : ''}`}
      aria-selected={value === item.value} aria-controls={item.controls}
      tabIndex={focused === item.value ? 0 : -1} onFocus={() => setFocused(item.value)}
      onClick={event => void activate(item.value, event.detail === 0)} onKeyDown={event => {
        if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
        event.preventDefault();
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1
          : (index + (event.key === 'ArrowRight' ? 1 : -1) + items.length) % items.length;
        event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next]?.focus();
        if (activation === 'automatic') void onChange(items[next].value);
      }}>{item.label}</button>)}
  </div>;
}
