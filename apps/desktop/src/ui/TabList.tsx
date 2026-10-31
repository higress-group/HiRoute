import { useEffect, useState, type ReactNode } from 'react';

type TabItem<T extends string> = { value: T; label: ReactNode; id?: string; controls?: string };

/** Arrow keys move focus. Guarded editors activate only on click/Enter/Space. */
export function TabList<const T extends string>({ items, value, onChange, label, className = '', activation = 'manual' }: {
  items: readonly TabItem<T>[];
  value: T;
  onChange(value: T): void;
  label: string;
  className?: string;
  activation?: 'manual' | 'automatic';
}) {
  const [focused, setFocused] = useState(value);
  useEffect(() => setFocused(value), [value]);
  return <div className={`tab-list ${className}`.trim()} role="tablist" aria-label={label}
    onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget)) setFocused(value); }}>
    {items.map((item, index) => <button key={item.value} id={item.id} type="button" role="tab"
      className={`segment${value === item.value ? ' active' : ''}`}
      aria-selected={value === item.value} aria-controls={item.controls}
      tabIndex={focused === item.value ? 0 : -1} onFocus={() => setFocused(item.value)}
      onClick={() => onChange(item.value)} onKeyDown={event => {
        if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
        event.preventDefault();
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1
          : (index + (event.key === 'ArrowRight' ? 1 : -1) + items.length) % items.length;
        event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next]?.focus();
        if (activation === 'automatic') onChange(items[next].value);
      }}>{item.label}</button>)}
  </div>;
}
