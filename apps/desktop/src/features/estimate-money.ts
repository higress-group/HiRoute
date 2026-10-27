import { formatMoney } from '../ui/money.ts';

/** Format integer micros without losing small values or narrowing a range. */
export function formatEstimateMoney(
  micros: number | null | undefined,
  currency: string | null | undefined,
  _language: 'zh' | 'en',
  options: { rounding?: 'nearest' | 'lower' | 'upper'; signed?: boolean } = {},
): string | null {
  if (!Number.isSafeInteger(micros) || typeof currency !== 'string') return null;
  const code = currency.trim().toUpperCase();
  if (!/^[A-Z]{3}$/.test(code)) return null;
  const value = BigInt(micros as number);
  if (value === 0n) return formatMoney(code, '0.00');
  const negative = value < 0n;
  const magnitude = negative ? -value : value;
  const decimals = magnitude < 100n ? 6 : magnitude < 1_000_000n ? 4 : 2;
  const step = 10n ** BigInt(6 - decimals);
  let units = magnitude / step;
  const remainder = magnitude % step;
  const rounding = options.rounding ?? 'nearest';
  if ((rounding === 'nearest' && remainder * 2n >= step)
    || (rounding === 'lower' && negative && remainder > 0n)
    || (rounding === 'upper' && !negative && remainder > 0n)) units += 1n;
  const digits = units.toString().padStart(decimals + 1, '0');
  const formatted = `${digits.slice(0, -decimals)}.${digits.slice(-decimals)}`;
  const sign = units === 0n ? '' : negative ? '−' : options.signed ? '+' : '';
  return sign + formatMoney(code, formatted);
}
