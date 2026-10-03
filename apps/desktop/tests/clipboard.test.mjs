import { test } from 'node:test';
import assert from 'node:assert/strict';
import { copyText } from '../src/ui/clipboard.ts';

function webview(t, clipboard, copied = true) {
  const calls = [];
  const input = { style: {}, value: '', setAttribute() {}, select() { calls.push(['select', this.value]); }, remove() { calls.push(['remove']); } };
  t.mock.getter(globalThis, 'navigator', () => ({ clipboard }));
  const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
  Object.defineProperty(globalThis, 'document', { configurable: true, value: {
    activeElement: { focus() { calls.push(['focus']); } },
    createElement() { return input; }, body: { append() {} },
    execCommand(command) { calls.push([command, input.value]); return copied; },
  } });
  t.after(() => {
    if (previousDocument) Object.defineProperty(globalThis, 'document', previousDocument);
    else delete globalThis.document;
  });
  return calls;
}

test('copy succeeds through the clipboard API without changing focus', async t => {
  let actual;
  const calls = webview(t, { async writeText(value) { actual = value; } });
  await copyText('launch command');
  assert.equal(actual, 'launch command');
  assert.deepEqual(calls, []);
});

test('native WebViews with a missing or rejected clipboard API still copy the exact command', async t => {
  for (const clipboard of [undefined, { async writeText() { throw new Error('NotAllowedError'); } }]) {
    await t.test(clipboard ? 'rejected' : 'missing', async sub => {
      const calls = webview(sub, clipboard);
      const command = `CODEX_HOME='/path with spaces' codex --profile hiroute`;
      await copyText(command);
      assert.deepEqual(calls, [['select', command], ['copy', command], ['remove'], ['focus']]);
    });
  }
});

test('failed copying reports failure and removes the temporary field', async t => {
  const calls = webview(t, undefined, false);
  await assert.rejects(copyText('launch command'), /CLIPBOARD_UNAVAILABLE/);
  assert.deepEqual(calls.slice(-2), [['remove'], ['focus']]);
});
