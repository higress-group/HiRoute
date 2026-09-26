export const DEFAULT_REQUEST_TIMEOUT_MS = 3_600_000;

export function requestTimeoutError(requestMs: number, attemptMs: number, language: 'zh' | 'en'): string | null {
  if (Number.isSafeInteger(requestMs) && requestMs >= attemptMs && requestMs <= 3_600_000) return null;
  const minimumSeconds = Math.ceil(attemptMs / 1000);
  return language === 'zh'
    ? `单次请求最长等待须在 ${minimumSeconds}–3600 秒之间。`
    : `The request timeout must be between ${minimumSeconds} and 3600 seconds.`;
}
