// Admit the adapter's actual native spawn contract, independently of its release label.
import childProcess from "node:child_process";
import { syncBuiltinESMExports } from "node:module";
import { isAbsolute, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export function managedSpawn(original, launcher) {
  return function (command, args, options) {
    // Accept only the literal selected launcher and app-server. Never evaluate a shell.
    if (command === `"${launcher}" app-server`) {
      if (!args || Array.isArray(args) || args.shell !== true) throw new Error("unexpected launch form");
      return original.call(this, launcher, ["app-server"], { ...args, shell: false });
    }
    if (command !== launcher || !Array.isArray(args) || args.length !== 1
        || args[0] !== "app-server" || options?.shell) {
      throw new Error("selected native app-server launch capability unavailable");
    }
    return original.call(this, launcher, args, { ...options, shell: false });
  };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
try {
  const adapter = process.argv[2];
  const launcher = process.env.CODEX_PATH;
  if (!adapter || !isAbsolute(adapter) || !launcher || !isAbsolute(launcher)
    || process.platform !== "win32" || !process.env.HIROUTE_NATIVE_LAUNCH_DESCRIPTOR) {
    throw new Error("invalid managed launch");
  }
  childProcess.spawn = managedSpawn(childProcess.spawn, launcher);
  syncBuiltinESMExports();
  process.argv = [process.argv[0], adapter, ...process.argv.slice(3)];
  await import(pathToFileURL(adapter).href);
} catch {
  process.stderr.write("HiRoute Codex managed Windows adapter launch failed\n");
  process.exitCode = 1;
}
}
