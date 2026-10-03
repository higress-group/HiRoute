export type CodexMode = 'profile' | 'root';
export type CodexSurface = 'codex_cli' | 'codex_desktop';

/** Discovery facts remain stable when the user changes the connection mode. */
export function codexSurfaceFacts(mode: CodexMode, detected: readonly string[]) {
  return (['codex_desktop', 'codex_cli'] as const).map(surface => ({
    surface,
    detected: detected.includes(surface),
    applicable: mode === 'root' || surface === 'codex_cli',
  }));
}
