import { nativeClipboard } from '../host/index.ts';

export async function copyText(value: string): Promise<void> {
  if (await nativeClipboard(value)) return;
  const secure = typeof window === 'undefined' || window.isSecureContext !== false;
  if (!secure) return manualCopy(value);
  try {
    if (navigator.clipboard?.writeText) { await navigator.clipboard.writeText(value); return; }
  } catch { /* A denied browser API retains the established exact-text copy path. */ }
  const previousFocus = document.activeElement as HTMLElement | null;
  const input = document.createElement('textarea');
  input.value = value; input.className = 'clipboard-probe'; input.setAttribute('readonly', '');
  document.body.append(input);
  try { input.select(); if (!document.execCommand('copy')) throw new Error('CLIPBOARD_UNAVAILABLE'); }
  finally { input.remove(); previousFocus?.focus(); }
}
function manualCopy(value: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const previous = document.activeElement as HTMLElement | null;
    const zh = document.querySelector('.occami-root')?.getAttribute('lang')?.startsWith('zh');
    const dialog = document.createElement('dialog'); dialog.className = 'clipboard-manual';
    const instructions = document.createElement('p'); instructions.textContent = zh ? '浏览器不允许自动复制。请选中下面的内容，然后按 Ctrl+C（Mac 为 ⌘C）复制。' : 'Automatic copying is unavailable. Select the text below, then press Ctrl+C (⌘C on Mac).';
    const input = document.createElement('textarea'); input.value = value; input.readOnly = true; input.setAttribute('aria-label', zh ? '待复制内容' : 'Text to copy');
    const close = document.createElement('button'); close.type = 'button'; close.className = 'btn'; close.textContent = zh ? '关闭' : 'Close';
    let copied = false;
    const finish = () => { dialog.close(); dialog.remove(); previous?.focus(); if (copied) resolve(); else reject(new Error('CLIPBOARD_UNAVAILABLE')); };
    input.addEventListener('copy', () => { copied = true; window.setTimeout(finish, 0); }, { once: true });
    close.addEventListener('click', finish); dialog.addEventListener('cancel', event => { event.preventDefault(); finish(); });
    dialog.append(instructions, input, close); document.body.append(dialog); dialog.showModal(); input.focus(); input.select();
  });
}
