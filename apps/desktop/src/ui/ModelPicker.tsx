import { ProviderIcon } from './ProviderIcon';
import { useState } from 'react';
import { Dialog } from './Dialog';
import { UiIcon } from './UiIcon';

export type ModelChoice = { id: string; name: string; source: string; optionId?: string | null; unavailable?: string };

/** Selection belongs to the caller's draft, never to the filtered list. */
export function ModelPicker({ title, description, language, items, value, single = false, showUnavailable = false, onApply, onClose }: {
  title: string;
  description?: string;
  language: 'zh' | 'en';
  items: ModelChoice[];
  value: string[];
  single?: boolean;
  showUnavailable?: boolean;
  onApply(ids: string[]): void;
  onClose(): void;
}) {
  const text = (cn: string, en: string) => language === 'zh' ? cn : en;
  const [query, setQuery] = useState('');
  const needle = query.trim().toLocaleLowerCase();
  const eligible = items.filter(item => !item.unavailable);
  const remaining = (showUnavailable ? items : eligible).filter(item => !value.includes(item.id));
  const visible = remaining.filter(item => `${item.name} ${item.source}`.toLocaleLowerCase().includes(needle));
  return <Dialog open title={title} closeLabel={text('关闭候选模型', 'Close candidates')} onClose={onClose}
    description={description}
    footer={<button className="btn" type="button" onClick={onClose}>{text('关闭', 'Close')}</button>}>
    <label className="search-box"><span className="sr-only">{text('搜索候选模型', 'Search candidates')}</span><UiIcon name="search" /><input className="input" data-autofocus placeholder={text('搜索模型或来源', 'Search models or sources')} value={query} onChange={event => setQuery(event.target.value)} /></label>
    <div className="native-list v3-catalog catalog-picker">
      {visible.map(item => <div className="list-row" key={item.id}>
        <ProviderIcon optionId={item.optionId} language={language} />
        <span className="row-main"><span className="row-title">{item.name}</span><span className="row-meta">{item.source}</span>{item.unavailable && <span className="row-meta">{item.unavailable}</span>}</span>
        <button className="btn btn-primary" type="button" disabled={Boolean(item.unavailable)} onClick={() => onApply(single ? [item.id] : [...value, item.id])}>{text(single ? '选择' : '添加', single ? 'Select' : 'Add')}</button>
      </div>)}
      {!visible.length && <div className="empty-state" role="status"><div><span className="empty-icon"><UiIcon name="models" /></span><h3>{needle ? text('没有匹配项', 'No matches') : remaining.length === 0 && eligible.length ? text('可用模型已全部加入', 'All available models have been added') : text('还没有符合条件的模型', 'No eligible models yet')}</h3><p>{needle ? text('试试其他名称或来源。', 'Try another name or source.') : eligible.length ? text('关闭后可以调整候选顺序。', 'Close to adjust the candidate order.') : text('先连接一个满足要求的模型。', 'Connect a model that meets the requirements.')}</p></div></div>}
    </div>
  </Dialog>;
}
