import React, { type ReactNode, type ComponentPropsWithoutRef } from 'react';

export function AgentCapabilityRow({ title, summary, actions, children, ...attributes }: {
  title: string; summary: ReactNode; actions: ReactNode; children?: ReactNode;
} & Omit<ComponentPropsWithoutRef<'section'>, 'title'>) {
  return <section {...attributes} className="detail-section v3-agent-section agent-capability-row">
    <div className="agent-capability-copy"><h3>{title}</h3><p>{summary}</p></div>
    <div className="agent-capability-actions detail-section-head">{actions}</div>
    {children && <div className="agent-capability-extra">{children}</div>}
  </section>;
}
