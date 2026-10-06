// Keep the raster Agent marks inside the entry bundle. WebKit's Tauri custom
// protocol can fail to resolve emitted binary assets even though the HTML and
// JavaScript bundle are embedded in the application.
import codexDark from './assets/codex-dark.png?inline';
import codexLight from './assets/codex-light.png?inline';
import claudeCode from './assets/claude-code.svg';
import qoder from './assets/qoder.svg';
import pi from './assets/pi.svg';
import dsh from './assets/providers/deepseek.svg';

export type BrandKind = 'codex' | 'claude-code' | 'qoder' | 'pi' | 'dsh' | 'agent' | 'model';

export function agentBrandFromId(agentId: string): BrandKind {
  const normalized = agentId.toLowerCase();
  if (normalized.includes('codex')) return 'codex';
  if (normalized.includes('claude')) return 'claude-code';
  if (normalized.includes('qoder')) return 'qoder';
  if (normalized === 'agent_dsh_default') return 'dsh';
  if (normalized === 'agent_pi_default') return 'pi';
  return 'agent';
}

export function BrandIcon({
  kind,
  label,
  size = 'medium',
}: {
  kind: BrandKind;
  label?: string;
  size?: 'small' | 'medium' | 'large';
}) {
  const accessibility = label
    ? { role: 'img', 'aria-label': label }
    : { 'aria-hidden': true as const };
  const classes = `hr-brand-icon agent-avatar v3-brand-avatar${kind === 'agent' || kind === 'model' ? ' hr-brand-icon--fallback' : ''}`;

  if (kind === 'codex') {
    return (
      <span className={classes} data-size={size} data-brand="codex" {...accessibility}>
        <img className="hr-brand-icon__light" src={codexLight} alt="" />
        <img className="hr-brand-icon__dark" src={codexDark} alt="" />
      </span>
    );
  }

  if (kind === 'claude-code') {
    return (
      <span className={classes} data-size={size} data-brand="claude-code" {...accessibility}>
        <img src={claudeCode} alt="" />
      </span>
    );
  }

  if (kind === 'qoder' || kind === 'pi' || kind === 'dsh') {
    return <span className={classes} data-size={size} data-brand={kind} {...accessibility}>
      <img src={kind === 'pi' ? pi : kind === 'dsh' ? dsh : qoder} alt="" />
    </span>;
  }

  return (
    <span className={classes} data-size={size} data-brand={kind} {...accessibility}>
      <svg viewBox="0 0 48 48" fill="none" aria-hidden="true">
        <path d="M15 18.5a9 9 0 0 1 18 0v11a5.5 5.5 0 0 1-5.5 5.5h-7A5.5 5.5 0 0 1 15 29.5v-11Z" />
        <path d="M20 22.5h.01M28 22.5h.01M20 29h8M24 9v-3M11 23H7M41 23h-4" />
      </svg>
    </span>
  );
}
