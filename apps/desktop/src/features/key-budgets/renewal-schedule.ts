import type { BudgetDraft, BudgetPeriod } from './types';

type CalendarDate = { year: number; month: number; day: number };
type Schedule = { month: number; day: number; weekday: number; time: string };
const pad = (value: number) => String(value).padStart(2, '0');
const calendarString = ({ year, month, day }: CalendarDate) => `${String(year).padStart(4, '0')}-${pad(month)}-${pad(day)}`;
function calendarDate({ year, month, day }: CalendarDate) {
  const date = new Date(0);
  date.setUTCFullYear(year, month - 1, day);
  return date;
}
function calendarParts(date: Date): CalendarDate {
  return { year: date.getUTCFullYear(), month: date.getUTCMonth() + 1, day: date.getUTCDate() };
}
export function renewalMonthDays(month: number): number {
  // Leap day remains a valid yearly rule; the ledger clamps non-leap years.
  return calendarDate({ year: 2000, month: month + 1, day: 0 }).getUTCDate();
}
function anchorDate(anchor: string): CalendarDate {
  const [year, month, day] = anchor.slice(0, 10).split('-').map(Number);
  return { year, month, day };
}
export function renewalSchedule(draft: BudgetDraft): Schedule {
  const date = anchorDate(draft.anchor);
  return { month: date.month, day: date.day, weekday: calendarDate(date).getUTCDay(), time: draft.anchor.slice(11) };
}
function today(draft: BudgetDraft, now: Date): CalendarDate {
  try {
    const parts = new Intl.DateTimeFormat('en', { timeZone: draft.timezone, year: 'numeric', month: '2-digit', day: '2-digit' }).formatToParts(now);
    const part = (type: string) => Number(parts.find(value => value.type === type)!.value);
    return { year: part('year'), month: part('month'), day: part('day') };
  } catch {
    // A temporarily incomplete time zone must not crash the editor. Save still validates it.
    return anchorDate(draft.anchor);
  }
}
function withSchedule(draft: BudgetDraft, schedule: Schedule, now: Date): BudgetDraft {
  let date = today(draft, now);
  if (draft.period === 'weekly') {
    const day = calendarDate(date);
    day.setUTCDate(day.getUTCDate() - (day.getUTCDay() - schedule.weekday + 7) % 7);
    date = calendarParts(day);
  } else if (draft.period === 'monthly') {
    // Keep the chosen day as the original anchor, even in a short current month.
    // Saving a past anchor does not backfill grants: the ledger starts at the current index.
    while (schedule.day > calendarDate({ year: date.year, month: date.month + 1, day: 0 }).getUTCDate()) {
      date = calendarParts(calendarDate({ year: date.year, month: date.month, day: 0 }));
    }
    date.day = schedule.day;
  } else if (draft.period === 'yearly') {
    date = { year: date.year, month: schedule.month, day: Math.min(schedule.day, renewalMonthDays(schedule.month)) };
    while (date.day > calendarDate({ year: date.year, month: date.month + 1, day: 0 }).getUTCDate()) date.year -= 1;
  }
  return { ...draft, anchor: `${calendarString(date)}T${schedule.time}` };
}
export function changeRenewalPeriod(draft: BudgetDraft, period: BudgetPeriod, now = new Date()): BudgetDraft {
  return period === draft.period ? draft : withSchedule({ ...draft, period }, renewalSchedule(draft), now);
}
export function changeRenewalSchedule(draft: BudgetDraft, patch: Partial<Schedule>, now = new Date()): BudgetDraft {
  const schedule = renewalSchedule(draft);
  if (Object.entries(patch).every(([key, value]) => schedule[key as keyof Schedule] === value)) return draft;
  return withSchedule(draft, { ...schedule, ...patch }, now);
}
