import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import { generateLinuxInstallScript } from '../src/lib/install-script.mjs';

const standalone = {
  kind: 'standalone', platform: 'Linux', architecture: 'x86_64', target: 'x86_64-unknown-linux-gnu',
  format: 'tar.gz', distribution: 'unsigned',
  filename: 'hiroute-1.2.3-aaaaaaaaaaaa-x86_64-unknown-linux-gnu.tar.gz', sha256: 'c'.repeat(64), size: 456789,
  manifest_filename: 'hiroute-1.2.3-aaaaaaaaaaaa-x86_64-unknown-linux-gnu.tar.gz.json',
  manifest_sha256: 'd'.repeat(64), manifest_size: 3456,
};
const armStandalone = {
  ...standalone,
  architecture: 'aarch64',
  target: 'aarch64-unknown-linux-gnu',
  filename: 'hiroute-1.2.3-aaaaaaaaaaaa-aarch64-unknown-linux-gnu.tar.gz',
  sha256: 'e'.repeat(64),
  manifest_filename: 'hiroute-1.2.3-aaaaaaaaaaaa-aarch64-unknown-linux-gnu.tar.gz.json',
  manifest_sha256: 'f'.repeat(64),
};
const manifest = { schema: 'hiroute.website.releases/v2', releases: [{
  version: '1.2.3', channel: 'stable', published_at: '2026-09-21T00:00:00Z',
  notes: { zh: '版本。', en: 'Release.' }, artifacts: [standalone, armStandalone],
}] };

test('generated installer selects the immutable stable manifest and does not start services', () => {
  const script = generateLinuxInstallScript(manifest);
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'hiroute-install-script-'));
  const bin = path.join(root, 'bin');
  fs.mkdirSync(bin);
  const log = path.join(root, 'calls.log');
  fs.writeFileSync(path.join(bin, 'uname'), '#!/bin/sh\n[ "${1:-}" = "-s" ] && echo Linux || echo x86_64\n', { mode: 0o755 });
  fs.writeFileSync(path.join(bin, 'curl'), '#!/bin/sh\nprintf "curl %s\\n" "$*" >> "$INSTALL_TEST_LOG"\nwhile [ "$#" -gt 0 ]; do [ "$1" = "-o" ] && { shift; printf "# fixture\\n" > "$1"; exit 0; }; shift; done\nexit 1\n', { mode: 0o755 });
  fs.writeFileSync(path.join(bin, 'python3'), '#!/bin/sh\nprintf "python3 %s\\n" "$*" >> "$INSTALL_TEST_LOG"\n', { mode: 0o755 });
  const result = spawnSync('/bin/sh', [], {
    input: script, encoding: 'utf8', env: {
      ...process.env, PATH: `${bin}:/usr/bin:/bin`, INSTALL_TEST_LOG: log, TMPDIR: root,
    },
  });
  assert.equal(result.status, 0, result.stderr);
  const calls = fs.readFileSync(log, 'utf8');
  assert.match(calls, /curl .*https:\/\/hiroute\.ai\/install\/standalone\.py/);
  assert.ok(calls.includes(`install --manifest-url https://hiroute.ai/releases/1.2.3/${standalone.manifest_filename}`));
  assert.ok(script.includes(`https://hiroute.ai/releases/1.2.3/${armStandalone.manifest_filename}`));
  assert.doesNotMatch(calls, /service start/);
  assert.match(result.stdout, /No service was started automatically/);
});

test('generated installer fails closed when no stable package exists for the host', () => {
  const script = generateLinuxInstallScript({ schema: manifest.schema, releases: [] });
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'hiroute-install-script-'));
  try {
    fs.writeFileSync(path.join(root, 'uname'), '#!/bin/sh\n[ "${1:-}" = "-s" ] && echo Linux || echo x86_64\n', { mode: 0o755 });
    const result = spawnSync('/bin/sh', [], {
      input: script, encoding: 'utf8', env: { ...process.env, PATH: `${root}:/usr/bin:/bin` },
    });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /No stable HiRoute package is published/);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
