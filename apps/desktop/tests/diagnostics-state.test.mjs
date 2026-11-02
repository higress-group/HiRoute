import assert from 'node:assert/strict';
import test from 'node:test';

import {
  acceptResponse,
  acceptStatus,
  canSave,
  fixedFailureText,
  nativeFailureCode,
  openLogsState,
  saveRequest,
  settingsReadErrorText,
  shouldRequest,
} from '../src/features/diagnostics/diagnostics-state.ts';

const status = (revision, level, extra = {}) => ({
  schema: 'hiroute.diagnostic-status/v1',
  settings: { revision, level, ...extra },
  logs_directory_available: true,
});

test('a save carries the saved revision and only exists for a real change', () => {
  const current = status(4, 'info');
  assert.equal(saveRequest(current, 'info'), null);
  assert.equal(canSave(current, 'info'), false);
  assert.deepEqual(saveRequest(current, 'debug'), { expected_revision: 4, level: 'debug' });
  assert.equal(canSave(current, 'debug'), true);
  assert.equal(canSave(null, 'debug'), false);
});

test('a newer status wins and an older one never rolls the view back', () => {
  const current = status(4, 'debug');
  assert.equal(acceptStatus(current, status(3, 'warn')), current);
  assert.equal(acceptStatus(current, status(5, 'warn')).settings.level, 'warn');
  assert.equal(acceptStatus(null, current), current);
});

test('the draft follows the accepted snapshot, never a rejected older response', () => {
  const current = status(4, 'debug');
  // A late poll that still carries revision 3 and a different level is rejected for the
  // status and for the draft alike.
  const late = acceptResponse(current, status(3, 'warn'), null);
  assert.equal(late.status, current);
  assert.equal(late.draft, 'debug');
  // A genuinely newer snapshot moves both.
  const newer = acceptResponse(current, status(5, 'warn'), null);
  assert.equal(newer.status.settings.revision, 5);
  assert.equal(newer.draft, 'warn');
  // The first response establishes the draft from the accepted snapshot.
  assert.equal(acceptResponse(null, current, null).draft, 'debug');
});

test('an unsaved draft survives polls and a save adopts the accepted level', () => {
  const saved = status(4, 'info');
  // The user picked debug and a poll arrives with the same revision: the choice stays.
  const polled = acceptResponse(saved, status(4, 'info'), 'debug');
  assert.equal(polled.draft, 'debug');
  assert.equal(polled.status.settings.revision, 4);
  assert.equal(polled.status.settings.level, 'info');
  // A poll with an older revision must not even replace the status behind the draft.
  const stale = acceptResponse(saved, status(2, 'error'), 'debug');
  assert.equal(stale.status, saved);
  assert.equal(stale.draft, 'debug');
  // After a save the draft returns to the accepted snapshot.
  const afterSave = acceptResponse(saved, status(5, 'debug'), null);
  assert.equal(afterSave.draft, 'debug');
  assert.equal(afterSave.status.settings.revision, 5);
});

test('a rejected read leaves the last accepted settings view in place', () => {
  const current = status(1, 'info');
  // A damaged file reported by the native side carries an error and keeps the last known
  // revision/level; the view follows those values instead of resetting the page.
  const damaged = acceptStatus(current, status(1, 'info', { error: 'settings_invalid' }));
  assert.equal(damaged.settings.error, 'settings_invalid');
  assert.equal(damaged.settings.level, 'info');
  assert.equal(damaged.settings.revision, 1);
});

test('a settings read error is announced as a read failure and clears on recovery', () => {
  const reading = status(1, 'info');
  assert.equal(settingsReadErrorText(reading, 'zh'), null);
  assert.equal(settingsReadErrorText(null, 'zh'), null);
  const damaged = acceptStatus(reading, status(1, 'info', { error: 'settings_invalid' }));
  assert.equal(
    settingsReadErrorText(damaged, 'zh'),
    '本机设置读取错误：本机设置文件已损坏，未覆盖。',
  );
  assert.equal(
    settingsReadErrorText(damaged, 'en'),
    'Could not read the local settings: The local settings file is damaged; it was not overwritten.',
  );
  // The recovered snapshot keeps the last known level and revision and drops the hint.
  const recovered = acceptStatus(damaged, status(1, 'info'));
  assert.equal(settingsReadErrorText(recovered, 'zh'), null);
  assert.equal(recovered.settings.level, 'info');
  assert.equal(recovered.settings.revision, 1);
});

test('failures map to fixed localized text instead of raw codes', () => {
  assert.equal(
    fixedFailureText('settings_conflict', 'zh'),
    '已有更新的保存，请刷新后重试。',
  );
  assert.equal(fixedFailureText('settings_conflict', 'en'), 'A newer save exists; refresh and try again.');
  assert.equal(
    fixedFailureText('override_active', 'zh'),
    '当前为临时覆盖等级，未保存。',
  );
  // An unknown code still yields fixed text, never the raw value.
  assert.equal(fixedFailureText('E_RAW_BACKEND_THING', 'en'), 'The action failed');
});

test('native rejections carry their lowercase code, other errors use the fallback', () => {
  assert.equal(nativeFailureCode({ source: 'native', code: 'override_active' }, 'x'), 'override_active');
  assert.equal(nativeFailureCode({ code: 'path_unsafe' }, 'x'), 'path_unsafe');
  // A wrong shape, an upper-case business code or a huge string never leaks into the text.
  assert.equal(nativeFailureCode('override_active', 'fallback'), 'fallback');
  assert.equal(nativeFailureCode({ code: 'E_RAW_BACKEND' }, 'fallback'), 'fallback');
  assert.equal(nativeFailureCode({ code: 'a'.repeat(200) }, 'fallback'), 'fallback');
  assert.equal(nativeFailureCode(null, 'fallback'), 'fallback');
});

test('opening the log directory reports one fixed result', () => {
  assert.deepEqual(openLogsState(true, undefined, 'zh'), {
    phase: 'opened',
    code: '已在本机文件管理器中打开日志目录。',
  });
  assert.equal(openLogsState(false, 'diagnostics_unavailable', 'zh').code, '诊断日志当前不可用。');
  assert.equal(openLogsState(false, 'path_unsafe', 'en').phase, 'failed');
});

test('requests are single-flight and only while the page is active', () => {
  assert.equal(shouldRequest(true, false), true);
  assert.equal(shouldRequest(true, true), false);
  assert.equal(shouldRequest(false, false), false);
});
