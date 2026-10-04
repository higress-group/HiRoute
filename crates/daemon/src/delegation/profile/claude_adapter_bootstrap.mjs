// This bootstrap runs only in the selected Claude ACP adapter process. It is not a
// preload and does not propagate a JavaScript guard to the native CLI or its tools.
import { isAbsolute } from "node:path";
import { pathToFileURL } from "node:url";

const fixedNames = new Set([
  "HOME", "PATH", "PWD", "TMPDIR", "TMP", "TEMP",
  "CLAUDE_CONFIG_DIR", "CLAUDE_CODE_EXECUTABLE",
  "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST",
  "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
  "CLAUDE_CODE_AUTO_COMPACT_WINDOW", "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
  "NODE_OPTIONS", "NODE_PATH", "BUN_OPTIONS",
  "LD_PRELOAD", "LD_LIBRARY_PATH", "DYLD_INSERT_LIBRARIES", "DYLD_LIBRARY_PATH",
]);

const transportNames = new Set([
  "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY",
  "http_proxy", "https_proxy", "all_proxy", "no_proxy",
  "NODE_EXTRA_CA_CERTS", "NODE_TLS_REJECT_UNAUTHORIZED", "NODE_USE_ENV_PROXY",
  "NODE_USE_SYSTEM_CA", "SSL_CERT_FILE", "SSL_CERT_DIR",
  "CURL_CA_BUNDLE", "REQUESTS_CA_BUNDLE",
  "CLAUDE_CODE_CLIENT_CERT", "CLAUDE_CODE_CLIENT_KEY",
  "CLAUDE_CODE_CLIENT_KEY_PASSPHRASE", "CLAUDE_CODE_CERT_STORE",
]);

class ManagedTransportConflict extends Error {}

function checkTransportWrite(target, key, value) {
  // Managed policy outranks the native flag layer and is read again by the CLI.
  // Swallowing a conflicting adapter write would leave that later bypass intact.
  if (transportNames.has(key) && target[key] !== value) {
    throw new ManagedTransportConflict();
  }
}

function hostControlled(name) {
  return typeof name === "string" && (
    fixedNames.has(name)
    || transportNames.has(name)
    || name.startsWith("ANTHROPIC_")
    || name.startsWith("CLAUDE_CODE_USE_")
    || name.startsWith("CLAUDE_CODE_OAUTH_")
    || name.startsWith("CLAUDE_CODE_API_KEY")
  );
}

// Protect absent keys too: managed settings must not introduce another credential,
// provider backend or runtime loader. Other settings env values remain native inputs.
const guardedEnvironment = new Proxy(process.env, {
  set(target, key, value) {
    if (transportNames.has(key)) checkTransportWrite(target, key, String(value));
    return hostControlled(key) || Reflect.set(target, key, value);
  },
  deleteProperty(target, key) {
    checkTransportWrite(target, key, undefined);
    return hostControlled(key) || Reflect.deleteProperty(target, key);
  },
  defineProperty(target, key, descriptor) {
    if (transportNames.has(key)) {
      if (!("value" in descriptor)) throw new ManagedTransportConflict();
      checkTransportWrite(target, key, String(descriptor.value));
    }
    return hostControlled(key) || Reflect.defineProperty(target, key, descriptor);
  },
});
Object.defineProperty(process, "env", {
  value: guardedEnvironment,
  writable: false,
  configurable: false,
  enumerable: true,
});

try {
  const adapter = process.argv[2];
  if (!adapter || !isAbsolute(adapter)) throw new Error("invalid adapter entry");
  process.argv = [process.argv[0], adapter, ...process.argv.slice(3)];
  await import(pathToFileURL(adapter).href);
} catch (error) {
  // Never echo an adapter error, environment value, or credential in this diagnostic.
  process.stderr.write(error instanceof ManagedTransportConflict
    ? "HiRoute Claude managed transport settings conflict\n"
    : "HiRoute Claude adapter bootstrap failed\n");
  process.exitCode = 1;
}
