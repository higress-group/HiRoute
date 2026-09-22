export type ContextWindowBounds = { maximum_tokens: number; default_tokens: number };

export function contextWindowError(value: number | undefined, bounds: ContextWindowBounds | null | undefined, language: 'zh' | 'en'): string | null {
  if (value === undefined) return null;
  const zh = language === 'zh';
  if (!Number.isSafeInteger(value) || value < 1) return zh ? '上下文窗口必须是正整数。' : 'The context window must be a positive integer.';
  if (!bounds) return zh ? '尚未确认候选共同窗口上界，请完成模型配置后重试。' : 'The shared context limit is not yet known. Complete the model configuration and try again.';
  if (value > bounds.maximum_tokens) return zh ? `上下文窗口不能超过候选共同上界 ${bounds.maximum_tokens.toLocaleString()} tokens。` : `The context window cannot exceed the shared candidate limit of ${bounds.maximum_tokens.toLocaleString()} tokens.`;
  return null;
}
