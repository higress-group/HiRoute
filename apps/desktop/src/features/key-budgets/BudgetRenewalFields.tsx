import React, { useId } from 'react';
import type { BudgetDraft } from './types';
import { changeRenewalPeriod, changeRenewalSchedule, renewalMonthDays, renewalSchedule } from './renewal-schedule';

export function BudgetRenewalFields({ language, value, onChange, disabled }: {
  language: 'zh' | 'en'; value: BudgetDraft; onChange(value: BudgetDraft): void; disabled?: boolean;
}) {
  const id = useId(), zh = language === 'zh', t = (cn: string, en: string) => zh ? cn : en;
  const schedule = renewalSchedule(value);
  const days = value.period === 'yearly' ? renewalMonthDays(schedule.month) : 31;
  const weekdayNames = zh ? ['星期日', '星期一', '星期二', '星期三', '星期四', '星期五', '星期六'] : ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
  const dayField = <label className="field" htmlFor={`${id}-day`}><span className="field-label">{t('补充日期', 'Renewal day')}</span>
    <select id={`${id}-day`} className="select" data-budget-day value={schedule.day} disabled={disabled} onChange={event => onChange(changeRenewalSchedule(value, { day: Number(event.target.value) }))}>
      {Array.from({ length: days }, (_, index) => index + 1).map(day => <option key={day} value={day}>{zh ? `${day} 日` : day}</option>)}
    </select>
  </label>;
  return <>
    <div className="key-budget-grid">
      <label className={`field${value.period === 'daily' ? '' : ' key-budget-period'}`} htmlFor={`${id}-period`}><span className="field-label">{t('补充周期', 'Renewal period')}</span>
        <select id={`${id}-period`} className="select" data-budget-period value={value.period} disabled={disabled} onChange={event => onChange(changeRenewalPeriod(value, event.target.value as BudgetDraft['period']))}>
          <option value="daily">{t('每天', 'Daily')}</option><option value="weekly">{t('每周', 'Weekly')}</option><option value="monthly">{t('每月', 'Monthly')}</option><option value="yearly">{t('每年', 'Yearly')}</option>
        </select>
      </label>
      {value.period === 'weekly' && <label className="field" htmlFor={`${id}-weekday`}><span className="field-label">{t('补充星期', 'Renewal weekday')}</span>
        <select id={`${id}-weekday`} className="select" data-budget-weekday value={schedule.weekday} disabled={disabled} onChange={event => onChange(changeRenewalSchedule(value, { weekday: Number(event.target.value) }))}>
          {[1, 2, 3, 4, 5, 6, 0].map(day => <option key={day} value={day}>{weekdayNames[day]}</option>)}
        </select>
      </label>}
      {value.period === 'monthly' && dayField}
      {value.period === 'yearly' && <div className="key-budget-month-day">
        <label className="field" htmlFor={`${id}-month`}><span className="field-label">{t('补充月份', 'Renewal month')}</span>
          <select id={`${id}-month`} className="select" data-budget-month value={schedule.month} disabled={disabled} onChange={event => onChange(changeRenewalSchedule(value, { month: Number(event.target.value) }))}>
            {Array.from({ length: 12 }, (_, index) => index + 1).map(month => <option key={month} value={month}>{zh ? `${month} 月` : month}</option>)}
          </select>
        </label>{dayField}
      </div>}
      <label className="field" htmlFor={`${id}-time`}><span className="field-label">{t('补充时间', 'Renewal time')}</span>
        <input id={`${id}-time`} className="input" data-budget-time type="time" value={schedule.time} disabled={disabled} onChange={event => onChange(changeRenewalSchedule(value, { time: event.target.value }))} />
      </label>
    </div>
    <p className="field-help">{t('时区：', 'Time zone: ')}{value.timezone}
      {value.period === 'monthly' && schedule.day > 28 && t('；短月按月末补充。', '; short months use their last day.')}
      {value.period === 'yearly' && schedule.month === 2 && schedule.day === 29 && t('；非闰年在 2 月 28 日补充。', '; non-leap years use February 28.')}
    </p>
  </>;
}
