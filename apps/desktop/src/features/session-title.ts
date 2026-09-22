const ENVIRONMENT_OPEN = '<environment_context>';
const ENVIRONMENT_CLOSE = '</environment_context>';
const REMINDER_OPEN = '<system-reminder>';
const REMINDER_CLOSE = '</system-reminder>';

export function isInjectedEnvironmentContext(value: string): boolean {
  const text = value.trim();
  if (!text.startsWith(ENVIRONMENT_OPEN)) return false;
  const close = text.indexOf(ENVIRONMENT_CLOSE, ENVIRONMENT_OPEN.length);
  return close >= 0 && close + ENVIRONMENT_CLOSE.length === text.length;
}

// Presentation only: keep every captured byte available behind the disclosure.
export function splitLeadingSystemReminders(value: string): { context: string; body: string } {
  let body = value;
  while (body.trimStart().startsWith(REMINDER_OPEN)) {
    const leading = body.length - body.trimStart().length;
    const close = body.indexOf(REMINDER_CLOSE, leading + REMINDER_OPEN.length);
    if (close < 0) break;
    body = body.slice(close + REMINDER_CLOSE.length);
  }
  return { context: value.slice(0, value.length - body.length), body };
}

function completeBlock(value: string, start: number, tag: string): number | null {
  const open = `<${tag}>`;
  if (!value.startsWith(open, start)) return null;
  const close = `</${tag}>`;
  const end = value.indexOf(close, start + open.length);
  return end < 0 ? null : end + close.length;
}

// Codex sends these structured setup blocks as user-role input to the Gateway.
// Their exact, complete leading boundaries are presentation context; captured
// bytes remain available, and free text after a block remains a user question.
export function splitLeadingAgentContext(value: string): { context: string; body: string } {
  const reminders = splitLeadingSystemReminders(value);
  const source = reminders.body;
  let end = 0;
  let codexBlock = false;
  while (true) {
    const start = end + (source.slice(end).match(/^\s*/u)?.[0].length ?? 0);
    const plugins = completeBlock(source, start, 'recommended_plugins');
    if (plugins !== null) { end = plugins; codexBlock = true; continue; }
    const heading = source.slice(start).match(/^# AGENTS\.md instructions(?: for [^\r\n]+)?\r?\n\s*<INSTRUCTIONS>/u);
    if (heading) {
      const instructions = completeBlock(source, start + heading[0].length - '<INSTRUCTIONS>'.length, 'INSTRUCTIONS');
      if (instructions !== null) { end = instructions; codexBlock = true; continue; }
    }
    const environment = completeBlock(source, start, 'environment_context');
    if (environment !== null && (codexBlock || isInjectedEnvironmentContext(source.slice(start)))) {
      end = environment;
      continue;
    }
    break;
  }
  return {
    context: reminders.context + source.slice(0, end),
    body: source.slice(end),
  };
}

export function firstRealUserText(values: readonly string[]): string | null {
  for (const value of values) {
    // Native clients can prepend complete setup blocks before the real
    // question. Keep captured bytes intact and select only the readable body.
    const title = splitLeadingAgentContext(value).body;
    if (!title.trim() || isInjectedEnvironmentContext(title)) continue;
    return title;
  }
  return null;
}
