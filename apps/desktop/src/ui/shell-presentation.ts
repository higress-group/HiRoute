import type { LanguagePreference, ThemePreference } from './preferences.ts';

export type ShellPresentation = { language: LanguagePreference; theme: ThemePreference };
const changed = 'hiroute-shell-presentation';
let presentation: ShellPresentation = { language: 'system', theme: 'system' };
let write: ((value: ShellPresentation) => Promise<void>) | undefined;
let pending = Promise.resolve();

function publish(value: ShellPresentation): void {
  presentation = value;
  window.dispatchEvent(new Event(changed));
}

export function bindShellPresentation(value: ShellPresentation | undefined, persist: (value: ShellPresentation) => Promise<void>): void {
  write = persist;
  if (value) publish(value);
}

export function currentShellPresentation(): ShellPresentation { return presentation; }
export function listenShellPresentation(listener: () => void): () => void {
  window.addEventListener(changed, listener);
  return () => window.removeEventListener(changed, listener);
}
export function updateShellPresentation(patch: Partial<ShellPresentation>): void {
  const value = { ...presentation, ...patch };
  publish(value);
  const persist = write;
  if (persist) {
    pending = pending.then(() => persist(value)).catch(() => {
      window.dispatchEvent(new Event('hiroute-shell-presentation-unavailable'));
    });
  }
}
