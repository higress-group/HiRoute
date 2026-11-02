import assert from "node:assert/strict";
import test from "node:test";
import { managedSpawn } from "./codex_adapter_bootstrap.mjs";

test("native argv contract works independently of adapter release or bundle bytes", () => {
  const launcher = "C:\\Program Files\\HiRoute\\hirouted.exe";
  const calls = [];
  const spawn = managedSpawn((...args) => { calls.push(args); return "child"; }, launcher);
  const options = { stdio: "pipe", env: { TEST: "retained" } };
  assert.equal(spawn(launcher, ["app-server"], options), "child");
  assert.deepEqual(calls, [[launcher, ["app-server"], { ...options, shell: false }]]);
});

test("literal shell producer is normalized without executing a shell", () => {
  const launcher = "C:\\HiRoute & tools\\hirouted.exe";
  let actual;
  const spawn = managedSpawn((...args) => { actual = args; }, launcher);
  spawn(`"${launcher}" app-server`, { shell: true, stdio: "pipe" });
  assert.deepEqual(actual, [launcher, ["app-server"], { shell: false, stdio: "pipe" }]);
});

test("changed command, argv and shell semantics fail before any native spawn", () => {
  const launcher = "C:\\HiRoute\\hirouted.exe";
  let calls = 0;
  const spawn = managedSpawn(() => { calls++; }, launcher);
  for (const args of [
    ["codex", ["app-server"]],
    [launcher, ["app-server", "--other"]],
    [launcher, ["app-server"], { shell: true }],
    [launcher, ["login"]],
    [`"${launcher}" app-server & other`, { shell: true }],
    [`"${launcher}" app-server`, { shell: false }],
  ]) assert.throws(() => spawn(...args));
  assert.equal(calls, 0);
});
