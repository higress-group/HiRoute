import type { PropsWithChildren, ReactNode } from 'react';

export type ProductPageProps = PropsWithChildren<{
  title: string;
  subtitle?: string;
  actions?: ReactNode;
  tabs?: ReactNode;
  flush?: boolean;
  className?: string;
}>;

/**
 * Shared V3 page frame. Keeping the toolbar and scrolling boundary in one
 * component prevents individual data states from changing the product grid.
 */
export function ProductPage({
  title,
  subtitle,
  actions,
  tabs,
  flush = false,
  className = '',
  children,
}: ProductPageProps) {
  return (
    <section className={`page ${className}`.trim()}>
      <header className="page-toolbar">
        <div className="page-title-wrap">
          <h1 className="page-title">{title}</h1>
          {subtitle && <div className="page-subtitle">{subtitle}</div>}
        </div>
        <div className="toolbar-actions">{actions}</div>
      </header>
      {tabs}
      <div className={`page-content${flush ? ' no-padding' : ''}`}>
        {children}
      </div>
    </section>
  );
}
