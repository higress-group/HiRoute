// Use the installed Desktop carrier and its real Host. The fixture observes
// the native model catalog and sends model calls through the persisted profile.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

export const inject = ["appReady", "llm"];

export function apply(ctx) {
  const { calls, absent, answer } = JSON.parse(process.env.HIROUTE_DSH_DESKTOP_PROBE);
  async function probe() {
    const registered = ctx.llm.listProviders().map(provider => provider.id);
    for (const provider of absent) assert(!registered.includes(provider));
    for (const { provider, model, prompt } of calls) {
      assert(registered.includes(provider));
      assert((await ctx.llm.listModels(provider)).some(entry => entry.id === model));
      const stream = await ctx.llm.stream({
        provider, model, maxTokens: 64,
        messages: [{ role: "user", content: [{ type: "text", text: prompt }] }],
      });
      const events = [];
      for await (const event of stream) events.push(event);
      assert(!events.some(event => event.type === "error"));
      assert(JSON.stringify(events).includes(answer));
    }
    process.send({ type: "hiroute-model-proof", calls: calls.length });
  }
  ctx.effect(() => ctx.appReady.onReady(() => {
    void probe().catch(error => process.send({
      type: "hiroute-model-proof", error: error.code ?? error.name,
    }));
  }));
}

async function run({ executable, resources, profile, home, dshHome, version, ...probe }) {
  const runtime = join(resources, "app.asar", "dsh");
  const host = join(runtime, "node_modules", "@deepseek-ai", "dsh-desktop-host", "lib", "index.js");
  const child = spawn(executable, ["--expose-internals", host, runtime, profile,
    join(resources, "runtime", "primary-runtime")], {
    cwd: profile, windowsHide: true,
    env: {
      ...process.env, HOME: home, USERPROFILE: home, APPDATA: home, LOCALAPPDATA: home,
      DSH_HOME: dshHome, DSH_CLIENT_VERSION: version, ELECTRON_RUN_AS_NODE: "1",
      DSH_TELEMETRY_DISABLED: "1", DSH_TELEMETRY_MODE: "DISABLED",
      HIROUTE_DSH_DESKTOP_PROBE: JSON.stringify(probe),
    },
    stdio: ["ignore", "ignore", "ignore", "ipc"],
  });
  let ready = false;
  let proof;
  let stopped = false;
  const exited = new Promise(resolve => child.once("exit", resolve));
  const finish = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", () => reject(new Error("Desktop Host exited before its model proof")));
    child.on("message", message => {
      if (message.type === "fatal") reject(new Error("Desktop Host failed to start"));
      if (message.type === "ready") ready = true;
      if (message.type === "hiroute-model-proof") proof = message;
      if (message.type === "shutdown-complete") stopped = true;
      if (ready && proof) resolve();
    });
  });
  const watchdog = setTimeout(() => child.kill(), 45_000);
  try {
    await finish;
    assert.equal(proof.error, undefined, "native model catalog or stream failed");
    assert.equal(proof.calls, probe.calls.length);
    child.send({ type: "shutdown" });
    assert.equal(await exited, 0);
    assert(stopped, "Desktop Host did not complete native shutdown");
    console.log(JSON.stringify({ calls: proof.calls, desktopHostReady: ready }));
  } finally {
    clearTimeout(watchdog);
    if (child.exitCode === null) child.kill();
  }
}

if (process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url) {
  const chunks = [];
  for await (const chunk of process.stdin) chunks.push(chunk);
  await run(JSON.parse(Buffer.concat(chunks).toString("utf8")));
}
