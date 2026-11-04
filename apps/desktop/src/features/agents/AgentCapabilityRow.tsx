import React, { type ReactNode, type ComponentPropsWithoutRef } from 'react';

export function AgentCapabilityRow({ title, summary, actions, children, ...attributes }: {
  title: string; summary: ReactNode; actions: ReactNode; children?: ReactNode;
} & Omit<ComponentPropsWithoutRef<'section'>, 'title'>) {
  return <section {...attributes} className="detail-section v3-agent-section agent-capability agent-capability-row">
    <div className="detail-section-head"><h3>{title}</h3><div className="agent-capability-actions">{actions}</div></div>
    <p>{summary}</p>
    {children && <div className="agent-capability-extra">{children}</div>}
  </section>;
}
