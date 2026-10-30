import test from 'node:test';
import assert from 'node:assert/strict';
import { nativeConnectionLost, nativeConnectionReadText } from '../src/native-connection.ts';

test('an explicit grant refusal does not present a lost native connection', () => {
  const error = { source: 'native', code: 'TRUSTED_AUTHORITY_BUSY' };
  assert.equal(nativeConnectionLost(error), false);
  assert.equal(nativeConnectionReadText(error, 'en'), undefined);
  assert.equal(nativeConnectionReadText(error, 'zh'), undefined);
});

test('uncertain or malformed exchanges retain connection recovery guidance', () => {
  for (const code of ['TRUSTED_AUTHORITY_UNAVAILABLE', 'PROTECTED_CHANNEL_TIMEOUT', 'PROTECTED_CHANNEL_CLOSED', 'REGISTRATION_ACK_INVALID', 'REGISTRATION_ACK_MISMATCH']) {
    assert.equal(nativeConnectionLost({ source: 'native', code }), true);
    assert.ok(nativeConnectionReadText({ source: 'native', code }, 'en'));
  }
});
