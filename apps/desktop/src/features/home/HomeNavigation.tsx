import { UiIcon, type Language } from '../../ui';
import type { HomeNavigationItem } from './types';
import hiRouteIcon from '../../../src-tauri/icons/icon.svg';

export function HiRouteMark() {
  return <span className="brand-mark" aria-hidden="true"><img src={hiRouteIcon} alt="" /></span>;
}

export function HomeNavigation({
  language,
  items,
  current,
  serviceTitle: providedServiceTitle,
  serviceLabel,
  serviceReady = false,
  onNavigate,
  onOpenSettings,
}: {
  language: Language;
  items: HomeNavigationItem[];
  current: string;
  serviceTitle?: string;
  serviceLabel?: string;
  serviceReady?: boolean;
  onNavigate: (id: string) => void;
  onOpenSettings?: () => void;
}) {
  const serviceTitle = providedServiceTitle ?? (serviceReady
    ? language === 'zh' ? '服务正常' : 'Service ready'
    : language === 'zh' ? '服务状态' : 'Service status');
  const settings = language === 'zh' ? '设置' : 'Settings';
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
          </button>
        ))}
      </nav>
      <div className="sidebar-spacer" />
      <div className="sidebar-foot">
        {serviceLabel && <div className="sidebar-health">
          <i className={`status-dot ${serviceReady ? 'good' : 'warn'}`} aria-hidden="true" />
          <div><strong>{serviceTitle}</strong><span>{serviceLabel}</span></div>
        </div>}
        {onOpenSettings && <button
          className={`nav-item ${current === 'settings' ? 'active' : ''}`}
          type="button"
          aria-current={current === 'settings' ? 'page' : undefined}
          aria-label={settings}
          title={settings}
          onClick={onOpenSettings}
        >
          <UiIcon name="settings" />
          <span>{settings}</span>
        </button>}
      </div>
    </aside>
  );
}
