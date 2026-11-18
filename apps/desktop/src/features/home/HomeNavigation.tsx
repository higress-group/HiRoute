import { useSyncExternalStore } from 'react';
import { HiRouteMark, UiIcon, type Language } from '../../ui';
import type { HomeNavigationItem } from './types';

// Match the collapsed sidebar breakpoint in shell.css.
const compactSidebarQuery = '(max-width: 1060px)';
const isCompactSidebar = () => window.matchMedia(compactSidebarQuery).matches;

function subscribeToSidebarLayout(onChange: () => void) {
  const layout = window.matchMedia(compactSidebarQuery);
  layout.addEventListener('change', onChange);
  return () => layout.removeEventListener('change', onChange);
}

export function HomeNavigation({
  language,
  items,
  current,
  serviceTitle: providedServiceTitle,
  serviceLabel,
  serviceReady = false,
  serviceCheckFailed = false,
  onRetryService,
  onNavigate,
  onOpenSettings,
  onOpenUpdates,
  routingAttention = false,
}: {
  language: Language;
  items: HomeNavigationItem[];
  current: string;
  serviceTitle?: string;
  serviceLabel?: string;
  serviceReady?: boolean;
  serviceCheckFailed?: boolean;
  onRetryService?: () => void;
  routingAttention?: boolean;
  onNavigate: (id: string) => void;
  onOpenSettings?: () => void;
  onOpenUpdates?: () => void;
}) {
  const serviceTitle = providedServiceTitle ?? (serviceReady
    ? language === 'zh' ? '服务正常' : 'Service ready'
    : language === 'zh' ? '服务状态' : 'Service status');
  const settings = language === 'zh' ? '设置' : 'Settings';
  const updateLabel = language === 'zh' ? '有新版本' : 'New version available';
  const compactSidebar = useSyncExternalStore(subscribeToSidebarLayout, isCompactSidebar);
  const compactUpdate = compactSidebar && Boolean(onOpenUpdates);
  const settingsLabel = compactUpdate ? `${settings} · ${updateLabel}` : settings;
  const openSettings = compactUpdate ? onOpenUpdates : onOpenSettings;

  return (
    <aside className="sidebar">
      <div className="brand">
        <HiRouteMark />
        <div className="brand-copy"><strong>HiRoute</strong><span>Agent routing</span></div>
      </div>
      <nav className="nav" aria-label={language === 'zh' ? '主导航' : 'Main navigation'}>
        {items.map(item => (
          <button
            key={item.id}
            className={`nav-item ${current === item.id ? 'active' : ''}`}
            aria-label={item.label}
            title={item.label}
            type="button"
            aria-current={current === item.id ? 'page' : undefined}
            onClick={() => onNavigate(item.id)}
          >
            <UiIcon name={item.icon} />
            <span>{item.label}</span>
            {item.id === 'routing' && routingAttention && <i className="status-dot warn trust-attention-dot" role="img" aria-label={language === 'zh' ? '有可选路由建议' : 'Optional routing suggestions'} />}
          </button>
        ))}
      </nav>
      <div className="sidebar-spacer" />
      <div className="sidebar-foot">
        {(serviceLabel || onOpenUpdates) && <div className="sidebar-health">
          {serviceLabel && <>
            <i className={`status-dot ${serviceReady ? 'good' : 'warn'}`} aria-hidden="true" />
            <div className="sidebar-health-copy"><strong>{serviceTitle}</strong><span>{serviceLabel}</span>{serviceCheckFailed && onRetryService && <button className="sidebar-health-retry" type="button" onClick={onRetryService}>{language === 'zh' ? '重试' : 'Retry'}</button>}</div>
          </>}
          {onOpenUpdates && <button className="sidebar-update-entry" type="button"
            aria-label={updateLabel} title={language === 'zh' ? '在设置中查看版本升级' : 'Open Updates in Settings'}
            onClick={onOpenUpdates}
          >
            <span className="sidebar-update-pill">{language === 'zh' ? updateLabel : 'Update'}<UiIcon name="chevronRight" /></span>
          </button>}
        </div>}
        {openSettings && <button
          className={`nav-item sidebar-settings${current === 'settings' ? ' active' : ''}`}
          type="button"
          aria-current={current === 'settings' ? 'page' : undefined}
          aria-label={settingsLabel}
          title={settingsLabel}
          onClick={openSettings}
        >
          <UiIcon name="settings" />
          <span>{settings}</span>
          {compactUpdate && <i className="sidebar-update-dot" aria-hidden="true" />}
        </button>}
      </div>
    </aside>
  );
}
