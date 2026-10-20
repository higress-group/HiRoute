export type QualityPeriod = 'all' | 'seven_days' | 'one_day';

const DAY_MS = 86_400_000;
const RETENTION_MS = 7 * DAY_MS;
// Both bounded reads must finish before their shared lower bound ages out.
const READ_MARGIN_MS = 60_000;

export function planQualityWindow(period: QualityPeriod, now: number) {
  return {
    from: Math.max(0, now - (period === 'one_day' ? DAY_MS : RETENTION_MS - READ_MARGIN_MS)),
    to: now + 1,
  };
}
