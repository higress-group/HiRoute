import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const bootstrap = fileURLToPath(new URL("./claude_adapter_bootstrap.mjs", import.meta.url));
const runtimeCredential = "synthetic-runtime-only-value";

function runAdapter(source, { guarded = true } = {}) {
  const root = mkdtempSync(join(tmpdir(), "hiroute-claude-bootstrap-"));
  try {
    const adapter = join(root, "selected adapter's entry.mjs");
    writeFileSync(adapter, source);
    return spawnSync(process.execPath, guarded ? [bootstrap, adapter] : [adapter], {
      encoding: "utf8",
      timeout: 5_000,
      env: {
        HOME: root, PATH: "/selected/runtime:/usr/bin:/bin", TMPDIR: root,
        CLAUDE_CONFIG_DIR: join(root, "native-context"),
        CLAUDE_CODE_EXECUTABLE: "/selected/native-cli",
        CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST: "1",
        CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
        ANTHROPIC_BASE_URL: "http://127.0.0.1:45678",
        ANTHROPIC_AUTH_TOKEN: runtimeCredential,
        ANTHROPIC_MODEL: "frozen-alias",
        ANTHROPIC_CUSTOM_MODEL_OPTION: "frozen-alias",
        NO_PROXY: "127.0.0.1", no_proxy: "127.0.0.1",
      },
    });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

function flags(result) {
  assert.equal(result.status, 0, "adapter fixture must complete");
  assert.equal(result.stderr, "", "adapter fixture must not emit diagnostics");
  assert.ok(!result.stdout.includes(runtimeCredential), "credential must not enter output");
  return JSON.parse(result.stdout);
}

const managedSettingsFixture = `
import { spawnSync } from 'node:child_process';
// macOS adds its own text-encoding variable after spawn; it is not a host-managed boundary.
const controlled = Object.keys(process.env).filter(key => !['NO_PROXY', 'no_proxy', '__CF_USER_TEXT_ENCODING'].includes(key));
const before = Object.fromEntries(controlled.map(key => [key, process.env[key]]));
const missing = [
  'ANTHROPIC_API_KEY', 'ANTHROPIC_DEFAULT_HAIKU_MODEL', 'CLAUDE_CODE_USE_BEDROCK',
  'CLAUDE_CODE_USE_VERTEX', 'CLAUDE_CODE_USE_FOUNDRY', 'CLAUDE_CODE_OAUTH_TOKEN',
  'CLAUDE_CODE_API_KEY_HELPER_TTL_MS', 'NODE_OPTIONS', 'NODE_PATH', 'LD_PRELOAD',
];
for (const key of [...controlled, ...missing]) {
  process.env[key] = 'managed-replacement';
  Object.assign(process.env, { [key]: 'assigned-replacement' });
  delete process.env[key];
  Object.defineProperty(process.env, key, {
    value: 'defined-replacement', configurable: true, enumerable: true, writable: true,
  });
}
process.env.SKILL_FIXTURE_SETTING = 'native-setting-kept';
const child = spawnSync(process.execPath, ['-e', [
  "const before = process.env.ANTHROPIC_MODEL === 'frozen-alias' && process.env.CLAUDE_CODE_EXECUTABLE === '/selected/native-cli' && process.env.CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST === '1' && !process.env.NODE_OPTIONS && !process.env.NODE_PATH",
  "process.env.ANTHROPIC_MODEL = 'child-local-value'",
  "process.stdout.write(JSON.stringify({before, unguarded: process.env.ANTHROPIC_MODEL === 'child-local-value'}))",
].join(';')], { encoding: 'utf8' });
const inherited = child.status === 0 ? JSON.parse(child.stdout) : {};
process.stdout.write(JSON.stringify({
  controlledPreserved: controlled.every(key => process.env[key] === before[key]),
  absentPreserved: missing.every(key => !(key in process.env)),
  ordinarySettingKept: process.env.SKILL_FIXTURE_SETTING === 'native-setting-kept',
  childReceivedManagedValues: inherited.before === true,
  guardNotPropagated: inherited.unguarded === true,
  selectedEntryPreserved: process.argv[1].endsWith("selected adapter's entry.mjs"),
}));
`;

test("managed settings cannot replace routing, context, selected CLI or absent provider credentials", () => {
  const observed = flags(runAdapter(managedSettingsFixture));
  assert.ok(Object.values(observed).every(value => value === true), "every managed boundary must hold");
});

test("the same settings fixture exposes the regression without the production bootstrap", () => {
  const observed = flags(runAdapter(managedSettingsFixture, { guarded: false }));
  assert.equal(observed.controlledPreserved, false);
  assert.equal(observed.absentPreserved, false);
  assert.equal(observed.childReceivedManagedValues, false);
});

test("replacing process.env fails before the adapter can continue and emits no credential", () => {
  const result = runAdapter(`
    process.env = { ...process.env, ANTHROPIC_BASE_URL: 'https://unmanaged.invalid' };
    process.stdout.write('adapter-continued');
  `);
  assert.equal(result.status, 1);
  assert.equal(result.stdout, "");
  assert.equal(result.stderr, "HiRoute Claude adapter bootstrap failed\n");
  assert.ok(!result.stderr.includes(runtimeCredential));
});

test("ordinary native settings remain writable and an import failure is sanitized", () => {
  const result = runAdapter(`
    process.env.NATIVE_SKILL_RESOURCE = 'enabled';
    if (process.env.NATIVE_SKILL_RESOURCE !== 'enabled') throw new Error('setting missing');
    throw new Error(process.env.ANTHROPIC_AUTH_TOKEN);
  `);
  assert.equal(result.status, 1);
  assert.equal(result.stdout, "");
  assert.equal(result.stderr, "HiRoute Claude adapter bootstrap failed\n");
  assert.ok(!result.stderr.includes(runtimeCredential));
});

function assertTransportConflict(result) {
  assert.equal(result.status, 1, "a managed transport conflict must reject startup");
  assert.equal(result.stdout, "", "a conflicting adapter must not continue");
  assert.equal(result.stderr, "HiRoute Claude managed transport settings conflict\n");
  assert.ok(!result.stderr.includes(runtimeCredential));
}

test("managed proxy and TLS injections fail before the native CLI can reread policy", () => {
  for (const key of [
    "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY",
    "http_proxy", "https_proxy", "all_proxy", "no_proxy",
    "NODE_EXTRA_CA_CERTS", "NODE_TLS_REJECT_UNAUTHORIZED", "NODE_USE_ENV_PROXY",
    "NODE_USE_SYSTEM_CA", "SSL_CERT_FILE", "SSL_CERT_DIR",
    "CURL_CA_BUNDLE", "REQUESTS_CA_BUNDLE",
    "CLAUDE_CODE_CLIENT_CERT", "CLAUDE_CODE_CLIENT_KEY",
    "CLAUDE_CODE_CLIENT_KEY_PASSPHRASE", "CLAUDE_CODE_CERT_STORE",
  ]) {
    assertTransportConflict(runAdapter(`
      process.env[${JSON.stringify(key)}] = process.env.ANTHROPIC_AUTH_TOKEN;
      process.stdout.write('adapter-continued');
    `));
  }
});

test("managed transport cannot delete, assign or redefine the Gateway proxy bypass", () => {
  for (const mutation of [
    "delete process.env.NO_PROXY",
    "Object.assign(process.env, { no_proxy: 'unmanaged.invalid' })",
    "Object.defineProperty(process.env, 'NO_PROXY', { value: 'unmanaged.invalid', configurable: true, enumerable: true, writable: true })",
    "Object.defineProperty(process.env, 'NO_PROXY', { get() { return 'unmanaged.invalid'; }, configurable: true })",
  ]) {
    assertTransportConflict(runAdapter(`${mutation}; process.stdout.write('adapter-continued');`));
  }
});

test("compatible managed bypass values reach the native child without a propagated guard", () => {
  const result = runAdapter(`
    import { spawnSync } from 'node:child_process';
    process.env.NO_PROXY = '127.0.0.1';
    Object.assign(process.env, { no_proxy: '127.0.0.1' });
    delete process.env.HTTP_PROXY;
    const child = spawnSync(process.execPath, ['-e', [
      "const bypass = process.env.NO_PROXY === '127.0.0.1' && process.env.no_proxy === '127.0.0.1'",
      "process.env.HTTP_PROXY = 'http://native-tool-proxy.invalid'",
      "process.stdout.write(JSON.stringify({bypass, toolProxyWritable: process.env.HTTP_PROXY === 'http://native-tool-proxy.invalid'}))",
    ].join(';')], { encoding: 'utf8' });
    if (child.status !== 0) throw new Error('child failed');
    process.stdout.write(child.stdout);
  `);
  assert.deepEqual(flags(result), { bypass: true, toolProxyWritable: true });
});
