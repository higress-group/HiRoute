import { useEffect, useMemo, useRef, useState } from 'react';
import { ProviderIcon } from '../../ui/ProviderIcon';
import { UiIcon } from '../../ui/UiIcon';
import { connectionTemplate } from '../../ui/provider-identity';
import { connectionDirectory, filterConnectionDirectory } from './connection-directory';
import type { BillingFilter } from './connection-directory';
import type { ComputeConnectionOption } from './types';

export function ConnectionDirectory({ options, loading, language, free, onSelect, onCustom, onReload, onInteraction }: {
  options: ComputeConnectionOption[];
  loading: boolean;
  language: 'zh' | 'en';
  free: boolean;
  onSelect: (option: ComputeConnectionOption) => void;
  onCustom?: () => void;
  onReload: () => void;
  onInteraction?: () => void;
}) {
  const zh = language === 'zh';
  const [query, setQuery] = useState('');
  const [billing, setBilling] = useState<BillingFilter>('all');
  const [providerId, setProviderId] = useState<string | null>(null);
  const content = useRef<HTMLElement>(null);
  const directory = useMemo(() => connectionDirectory(options, language, connectionTemplate), [options, language]);
  const visible = useMemo(() => filterConnectionDirectory(directory, query, billing), [directory, query, billing]);
  const provider = providerId ? visible.find(item => item.id === providerId) ?? null : null;
  const productCount = directory.reduce((sum, item) => sum + item.products.length, 0);
  useEffect(() => {
    if (loading) return;
    const frame = requestAnimationFrame(() => {
      content.current?.querySelector<HTMLElement>(providerId ? '.connection-product-card:not(:disabled), .connection-directory-back' : 'input[type=search]')?.focus();
      content.current?.closest('.modal-body')?.scrollTo({ top: 0 });
    });
    return () => cancelAnimationFrame(frame);
  }, [providerId, loading]);
  const billingLabel = (value: ComputeConnectionOption['billing_class']) => ({
    paid: zh ? '按量付费' : 'Pay as you go',
    subscription: zh ? '套餐' : 'Plan',
    free: zh ? '免费资源' : 'Free resources',
    unknown: zh ? '计费方式待确认' : 'Billing unconfirmed',
  })[value];

  if (loading) return <div className="oc-status-row" role="status"><span className="oc-spinner" /><div className="row-main"><strong>{zh ? '正在读取服务目录' : 'Loading services'}</strong><p>{zh ? '正在确认当前版本支持的接入方式。' : 'Checking connections supported by this version.'}</p></div></div>;
  if (providerId) return <section className="connection-directory" ref={content} onClickCapture={onInteraction} onInputCapture={onInteraction}>
    <button className="btn btn-quiet connection-directory-back" type="button" onClick={() => setProviderId(null)}><UiIcon name="arrowLeft" />{zh ? '所有供应商' : 'All providers'}</button>
    {provider ? <><div className="connection-product-heading"><ProviderIcon optionId={provider.products[0].option.connection_option_id} language={language} /><div><h3>{provider.name}</h3><p>{zh ? '选择与你的 API Key 对应的产品' : 'Choose the product associated with your API key'}</p></div></div>
    <div className="connection-product-list">{provider.products.map(product => <button className="connection-product-card" key={product.option.connection_option_id} type="button" disabled={!product.option.registered_check_available} onClick={() => onSelect(product.option)}>
      <span className="row-main"><strong>{product.name}</strong><span>{connectionTemplate(product.option.connection_option_id)?.description[language] || product.option.display_name}</span></span>
      <span className="connection-billing">{billingLabel(product.option.billing_class)}</span><UiIcon name="chevronRight" />
    </button>)}</div></> : <div className="connection-directory-empty" role="status"><p>{zh ? '此供应商当前不可用。' : 'This provider is currently unavailable.'}</p></div>}
  </section>;
  return <section className="connection-directory" ref={content} onClickCapture={onInteraction} onInputCapture={onInteraction}>
    <div className="connection-directory-intro"><h3>{free ? (zh ? '选择免费资源' : 'Choose free resources') : (zh ? '选择 API 服务' : 'Choose an API service')}</h3><p>{zh ? `当前支持 ${directory.length} 家供应商 · ${productCount} 种接入方式` : `${directory.length} providers · ${productCount} connection options`}</p></div>
    <label className="field"><span className="sr-only">{zh ? '搜索供应商或产品' : 'Search providers or products'}</span><span className="connection-search"><UiIcon name="search" /><input className="input" data-autofocus type="search" value={query} placeholder={zh ? '搜索供应商或产品，如百炼、Coding Plan' : 'Search providers or products, e.g. Coding Plan'} onChange={event => setQuery(event.target.value)} /></span></label>
    {!free && <div className="connection-billing-filters" role="group" aria-label={zh ? '计费方式' : 'Billing'}>{(['all', 'paid', 'subscription', 'free'] as const).map(value => <button type="button" key={value} aria-pressed={billing === value} onClick={() => setBilling(value)}>{value === 'all' ? (zh ? '全部' : 'All') : billingLabel(value)}</button>)}</div>}
    {visible.length ? <div className="connection-provider-grid">{visible.map(item => <button className="connection-provider-card" type="button" key={item.id} disabled={!item.products.some(product => product.option.registered_check_available)} onClick={() => item.products.length === 1 ? onSelect(item.products[0].option) : setProviderId(item.id)}>
      <ProviderIcon optionId={item.products[0].option.connection_option_id} language={language} /><strong>{item.name}</strong><span>{item.products.length > 1 ? (zh ? `${item.products.length} 种接入方式` : `${item.products.length} options`) : billingLabel(item.products[0].option.billing_class)}</span>
    </button>)}</div> : <div className="connection-directory-empty" role="status"><p>{options.length ? (zh ? '没有找到匹配的服务' : 'No matching service') : (zh ? '暂时无法读取服务目录' : 'The service directory is unavailable')}</p>{options.length ? <button type="button" className="btn" onClick={() => { setQuery(''); setBilling('all'); }}>{zh ? '清除筛选' : 'Clear filters'}</button> : <button type="button" className="btn" onClick={onReload}>{zh ? '重新读取' : 'Reload'}</button>}</div>}
    {(query || billing !== 'all') && <p className="field-help" role="status">{zh ? `显示 ${visible.length} / ${directory.length} 家供应商` : `Showing ${visible.length} of ${directory.length} providers`}</p>}
    {!free && onCustom && <button className="connection-custom-entry" type="button" onClick={onCustom}><UiIcon name="plus" /><span><strong>{zh ? '自定义 API' : 'Custom API'}</strong><span>{zh ? '连接代理服务、本地模型或其他兼容服务' : 'Connect a proxy, local models or another compatible service'}</span></span><UiIcon name="chevronRight" /></button>}
  </section>;
}
