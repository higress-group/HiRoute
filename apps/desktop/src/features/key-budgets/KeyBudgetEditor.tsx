import React, { useId } from 'react';
import './key-budgets.css';
import { Disclosure } from '../../ui';
import { budgetMoney, type BudgetDraft, type BudgetView } from './types';

/** Both entry points submit this draft through the source's single save action. */
export function KeyBudgetEditor({ language, value, onChange, disabled, current, shared = [] }: {
  language: 'zh' | 'en'; value: BudgetDraft; onChange(value: BudgetDraft): void; disabled?: boolean; current?: BudgetView; shared?: BudgetView[];
}) {
  const id = useId(), zh = language === 'zh', t = (cn: string, en: string) => zh ? cn : en;
  const update = (patch: Partial<BudgetDraft>) => onChange({ ...value, ...patch });
  const money = (amount: number) => budgetMoney(amount, current?.definition.policy.currency ?? value.currency, language);
  return <div className="key-budget-editor" data-key-budget>
    <label className="key-budget-toggle"><input type="checkbox" checked={value.enabled} disabled={disabled} onChange={event => update({ enabled: event.target.checked })} /><span>{t('限制 API 花费', 'Limit API spending')}</span></label>
    {current && <p className="field-help" role="status">{t('可用', 'Available')} {money(current.available_micros)} · {t('已用', 'Spent')} {money(current.lifetime_spent_micros)}{current.reserved_micros > 0 && <> · {t('待结算', 'Pending')} {money(current.reserved_micros)}</>}{current.key_count > 1 && <> · {t(`${current.key_count} 个 Key 共用`, `Shared by ${current.key_count} keys`)}</>}
      {current.next_renewal_at_ms !== null && <><br />{t('下次补充：', 'Next renewal: ')}{new Date(current.next_renewal_at_ms).toLocaleString(zh ? 'zh-CN' : 'en-US', { timeZone: current.definition.policy.renewal?.timezone })} {current.definition.policy.renewal?.timezone}</>}
    </p>}
    {value.enabled && <div className="key-budget-fields">
      <div className="key-budget-grid">
        <label className="field" htmlFor={`${id}-mode`}><span className="field-label">{t('额度方式', 'Allowance')}</span><select id={`${id}-mode`} className="select" value={value.mode} disabled={disabled} onChange={event => update({ mode: event.target.value as BudgetDraft['mode'] })}>
          <option value="once">{t('固定金额，用完即止', 'One-time amount')}</option><option value="periodic">{t('定期补充', 'Recurring allowance')}</option>{shared.length > 0 && <option value="shared">{t('共用已有额度', 'Share an existing allowance')}</option>}
        </select></label>
        {value.mode === 'shared' ? <label className="field"><span className="field-label">{t('共用额度', 'Shared allowance')}</span><select className="select" value={value.sharedId} disabled={disabled} onChange={event => { const item = shared.find(item => item.definition.budget_id === event.target.value); update({ sharedId: item?.definition.budget_id ?? '', sharedRevision: item?.definition.revision ?? 0 }); }}>
          <option value="">{t('选择额度', 'Choose an allowance')}</option>{shared.map(item => <option key={item.definition.budget_id} value={item.definition.budget_id}>{item.definition.display_name} · {budgetMoney(item.available_micros, item.definition.policy.currency, language)} · {item.definition.budget_id.slice(-6)}</option>)}
        </select></label> : <label className="field"><span className="field-label">{value.mode === 'once' ? t('总金额', 'Total amount') : t('每期金额', 'Amount per period')}</span><span className="key-budget-amount"><select className="select" aria-label={t('币种', 'Currency')} value={value.currency} disabled={disabled || Boolean(value.existing)} onChange={event => update({ currency: event.target.value })}><option>USD</option><option>CNY</option>{!['USD', 'CNY'].includes(value.currency) && <option>{value.currency}</option>}</select><input className="input" aria-label={t('金额', 'Amount')} inputMode="decimal" placeholder="100.00" value={value.amount} disabled={disabled} onChange={event => update({ amount: event.target.value })} /></span></label>}
      </div>
      {value.mode === 'periodic' && <>
        <div className="key-budget-grid"><label className="field"><span className="field-label">{t('补充周期', 'Renewal period')}</span><select className="select" value={value.period} disabled={disabled} onChange={event => update({ period: event.target.value as BudgetDraft['period'] })}>
          <option value="daily">{t('每天', 'Daily')}</option><option value="weekly">{t('每周', 'Weekly')}</option><option value="monthly">{t('每月', 'Monthly')}</option><option value="yearly">{t('每年', 'Yearly')}</option>
        </select></label><label className="field"><span className="field-label">{t('补充日期与时间', 'Renewal date and time')}</span><input className="input" type="datetime-local" value={value.anchor} disabled={disabled} onChange={event => update({ anchor: event.target.value })} /></label></div>
        <p className="field-help">{t('默认每期恢复到设定金额，余额不结转。月底日期在短月按月末执行。', 'Each period restores the amount without carryover. Month-end dates use the last available day.')}</p>
      </>}
      {value.mode !== 'shared' && <Disclosure language={language} label={t('更多额度设置', 'More allowance settings')}>
        <label className="field"><span className="field-label">{t('额度名称', 'Allowance name')}</span><input className="input" value={value.name} maxLength={256} disabled={disabled} onChange={event => update({ name: event.target.value })} /></label>
        {value.mode === 'periodic' && <>
          <label className="field"><span className="field-label">{t('时区', 'Time zone')}</span><input className="input" value={value.timezone} disabled={disabled} onChange={event => update({ timezone: event.target.value })} /></label>
          <label className="field"><span className="field-label">{t('余额规则', 'Balance rule')}</span><select className="select" value={value.renewalMode} disabled={disabled} onChange={event => update({ renewalMode: event.target.value as BudgetDraft['renewalMode'] })}><option value="reset">{t('恢复到每期金额', 'Restore the period amount')}</option><option value="accumulate">{t('累加，并限制余额上限', 'Accumulate up to a balance cap')}</option></select></label>
          {value.renewalMode === 'accumulate' && <label className="field"><span className="field-label">{t('余额上限', 'Balance cap')}</span><input className="input" inputMode="decimal" value={value.cap} disabled={disabled} onChange={event => update({ cap: event.target.value })} /></label>}
        </>}
        {!value.existing && <label className="field"><span className="field-label">{t('首次可用金额（选填）', 'Initial available amount (optional)')}</span><input className="input" inputMode="decimal" placeholder={value.amount || '100.00'} value={value.initial} disabled={disabled} onChange={event => update({ initial: event.target.value })} /><span className="field-help">{t('留空使用完整金额；已用过赠金时可填写剩余金额。', 'Blank starts with the full amount. Enter the remainder if credits were already spent.')}</span></label>}
      </Disclosure>}
      <p className="field-help">{t('仅统计经过 HiRoute 的预估花费，并非服务商余额。额度不足或价格未知时尝试其他 Key 或候选；在途花费先占用额度。', 'Tracks estimated spending through HiRoute, not provider balance. Insufficient allowance or unknown prices try the next key or candidate; in-flight costs are reserved.')}</p>
      {value.existing && <p className="field-help">{t('修改金额保留已用花费；共用此额度的 Key 同时生效。', 'Amount edits retain spending and apply to all keys sharing this allowance.')}</p>}
    </div>}
  </div>;
}
