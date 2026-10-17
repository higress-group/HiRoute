import type { PropsWithChildren } from 'react';
import type { Language, ResolvedTheme, TextScale } from './preferences';

export type PresentationRootProps = PropsWithChildren<{
  language: Language;
  theme: ResolvedTheme;
  textScale: TextScale;
  className?: string;
}>;

export function PresentationRoot({
  language,
  theme,
  textScale,
  className = '',
  children,
}: PresentationRootProps) {
  return (
    <div
      className={`hr-ui occami-root ${className}`.trim()}
      data-theme={theme}
      data-text-scale={String(textScale)}
      lang={language === 'zh' ? 'zh-CN' : 'en'}
    >
      {children}
    </div>
  );
}
