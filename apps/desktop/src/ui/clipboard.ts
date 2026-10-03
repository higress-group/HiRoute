import { isTauri } from '@tauri-apps/api/core';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';

export async function copyText(value: string): Promise<void> {
  if (isTauri()) {
    await writeText(value);
    return;
  }
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(value);
      return;
    }
  } catch {
    // Some native WebViews expose this API but cannot use it after an async save.
  }
  const previousFocus = document.activeElement as HTMLElement | null;
  const input = document.createElement('textarea');
  input.value = value;
  input.setAttribute('readonly', '');
  input.style.position = 'fixed';
  input.style.opacity = '0';
  document.body.append(input);
  try {
    input.select();
    if (!document.execCommand('copy')) throw new Error('CLIPBOARD_UNAVAILABLE');
  } finally {
    input.remove();
    previousFocus?.focus();
  }
}
