import { useState } from 'react';
import { ProviderIcon } from '../../ui/ProviderIcon';
import { connectionName } from '../../ui/provider-identity';
import type { ComputeConnectionOption, Language } from './types';

export function ConnectionTemplatePicker({ options, value, language, onChange }: {
  options: ComputeConnectionOption[];
  value: string;
  language: Language;
  onChange(id: string): void;
}) {
  const zh = language === 'zh';
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState<'all' | 'paid' | 'free'>('all');
  const [page, setPage] = useState(0);
  const needle = query.trim().toLocaleLowerCase();
  const matches = options.filter(option => (filter === 'all' || option.billing_class === filter)
    && `${connectionName(option.connection_option_id, language, option.display_name)} ${option.display_name} ${option.connection_option_id}`.toLocaleLowerCase().includes(needle));
  const pages = Math.max(1, Math.ceil(matches.length / 6));
  const currentPage = Math.min(page, pages - 1);
  return <section className="mc-template-directory" aria-label={zh ? '接入模板' : 'Connection templates'}>
    <label className="field"><span className="field-label">{zh ? '搜索接入模板' : 'Search connection templates'}</span><input className="input" type="search" placeholder={zh ? '供应商或产品，如百炼、Coding Plan' : 'Provider or product, such as Bailian or Coding Plan'} value={query} onChange={event => { setQuery(event.target.value); setPage(0); }} /></label>
    <div className="actions" role="group" aria-label={zh ? '模板类型' : 'Template type'}>{(['all', 'paid', 'free'] as const).map(kind => <button className="filter-chip" type="button" key={kind} aria-pressed={filter === kind} onClick={() => { setFilter(kind); setPage(0); }}>{kind === 'all' ? zh ? '全部' : 'All' : kind === 'paid' ? zh ? '付费 API' : 'Paid API' : zh ? '免费' : 'Free'}</button>)}</div>
    <div role="radiogroup" aria-label={zh ? '选择接入模板' : 'Choose a connection template'}>{matches.slice(currentPage * 6, (currentPage + 1) * 6).map(option => <label className={`mc-template-row${value === option.connection_option_id ? ' selected' : ''}`} key={option.connection_option_id}>
      <input type="radio" name="connection-template" value={option.connection_option_id} checked={value === option.connection_option_id} onChange={() => onChange(option.connection_option_id)} />
      <ProviderIcon optionId={option.connection_option_id} language={language} /><span className="row-main"><strong>{connectionName(option.connection_option_id, language, option.display_name)}</strong><span className="field-help">{[...new Set(option.endpoints?.map(endpoint => endpoint.protocol) ?? [])].join(' · ')}</span></span>
    </label>)}</div>
    {!matches.length && <p role="status">{zh ? '没有匹配的模板。可以使用自定义 API 手动填写。' : 'No matching templates. Use Custom API to enter connection settings.'}</p>}
    <div className="mc-template-pagination"><span>{matches.length} {zh ? '种接入模板' : 'templates'}</span><button className="btn btn-quiet" type="button" disabled={currentPage === 0} onClick={() => setPage(currentPage - 1)}>{zh ? '上一页' : 'Previous'}</button><span>{currentPage + 1}/{pages}</span><button className="btn btn-quiet" type="button" disabled={currentPage + 1 >= pages} onClick={() => setPage(currentPage + 1)}>{zh ? '下一页' : 'Next'}</button></div>
  </section>;
}
