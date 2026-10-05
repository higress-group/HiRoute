import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync, mkdirSync, writeFileSync, rmSync} from 'node:fs';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
import {loadPiSdk, assertPiCapabilities, probePiCapabilities} from './pi_sdk_contract.mjs';

function sdk() {
  class ModelRuntime {
    static async create(options) {
      assert.equal(options.allowModelNetwork, false);
      assert.equal(options.refreshOnCreate, false);
      assert.deepEqual(await options.credentials.list(), []);
      return new ModelRuntime();
    }
    getModels(provider) {return [{provider, id:'capability', api:'openai-responses',
      baseUrl:'http://127.0.0.1:1/v1', contextWindow:16384, maxTokens:4096}];}
    registerProvider() {} setRuntimeApiKey() {}
  }
  class SettingsManager {static create() {} static fromStorage() {} getGlobalSettings() {} getProjectSettings() {}}
  class DefaultResourceLoader {reload() {} getSkills() {}}
  class SessionManager {static create() {} static open() {} getSessionId() {} getSessionFile() {} buildSessionContext() {} setSessionFile() {}}
  class AgentSession {setModel() {} subscribe() {} prompt() {} clearQueue() {} abort() {} dispose() {}}
  return {ModelRuntime, SettingsManager, DefaultResourceLoader, SessionManager, AgentSession,
    createAgentSession(){}, createReadTool(){}, createBashTool(){}, CURRENT_SESSION_VERSION:3};
}

test('the selected CLI owns its SDK after a version or npm entry-layout change', async () => {
  for (const version of ['1.0.2', '1.0.3', '9.0.0-next']) {
    const root = mkdtempSync(join(tmpdir(), 'hiroute-pi-sdk-test-'));
    try {
      mkdirSync(join(root, 'bin')); mkdirSync(join(root, 'sdk'));
      const cli = join(root, 'bin/pi.js');
      // Importing the SDK must never execute the selected CLI.
      writeFileSync(cli, 'throw new Error("native CLI executed during admission");');
      writeFileSync(join(root, 'sdk/index.mjs'), 'export const selected = "owned-SDK";');
      const pkg = {name:'@earendil-works/pi-coding-agent', version,
        bin:{pi:'bin/pi.js'}, exports:{'.':{import:'./sdk/index.mjs'}}};
      writeFileSync(join(root, 'package.json'), JSON.stringify(pkg));
      const loaded = await loadPiSdk(cli);
      assert.equal(loaded.pkg.version, version);
      assert.equal(loaded.sdk.selected, 'owned-SDK');
      pkg.exports['.'].import = '../../foreign-sdk.mjs';
      writeFileSync(join(root, 'package.json'), JSON.stringify(pkg));
      await assert.rejects(loadPiSdk(cli));
      pkg.bin.pi = 'bin/other.js';
      writeFileSync(join(root, 'package.json'), JSON.stringify(pkg));
      await assert.rejects(loadPiSdk(cli));
    } finally {rmSync(root, {recursive:true, force:true});}
  }
});

test('model routing checks native declarations without requiring Worker or history interfaces', async () => {
  const value = sdk();
  delete value.createAgentSession; delete value.SessionManager; delete value.SettingsManager;
  await probePiCapabilities(value, 'models');
  value.ModelRuntime.prototype.getModels = () => [];
  await assert.rejects(probePiCapabilities(value, 'models'), /configuration unavailable/);
  delete value.ModelRuntime.create;
  assert.throws(() => assertPiCapabilities(value, 'models'), /capability unavailable/);
});

test('task-route configuration requires resources and tools, independently of Worker SDK', () => {
  const value = sdk(); delete value.ModelRuntime; delete value.createAgentSession;
  assertPiCapabilities(value, 'collaboration');
  delete value.createBashTool;
  assert.throws(() => assertPiCapabilities(value, 'collaboration'), /capability unavailable/);
});

test('a new Worker does not require native open; Continue does', () => {
  const value = sdk(); delete value.SessionManager.open;
  assertPiCapabilities(value, 'worker');
  assert.throws(() => assertPiCapabilities(value, 'continue'), /capability unavailable/);
  delete value.AgentSession.prototype.abort;
  assert.throws(() => assertPiCapabilities(value, 'worker'), /capability unavailable/);
  const resume = sdk(); delete resume.SessionManager.create; delete resume.SessionManager.prototype.setSessionFile;
  assertPiCapabilities(resume, 'continue');
  assert.throws(() => assertPiCapabilities(resume, 'worker'), /capability unavailable/);
});

test('an unknown SDK history writer is refused without blocking model or task-route setup', () => {
  const value = sdk(); value.CURRENT_SESSION_VERSION = 4;
  assertPiCapabilities(value, 'models'); assertPiCapabilities(value, 'collaboration');
  assert.throws(() => assertPiCapabilities(value, 'worker'), /session format unavailable/);
  assert.throws(() => assertPiCapabilities(value, 'continue'), /session format unavailable/);
});
