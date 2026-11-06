import { artifactManifestUrl, latestRelease, validateReleaseManifest } from './releases.mjs';

const TARGETS = ['x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu'];

function shellLiteral(value) {
  return `'${value.replaceAll("'", `'"'"'`)}'`;
}

export function generateLinuxInstallScript(manifest) {
  const stable = latestRelease('stable', validateReleaseManifest(manifest));
  const urls = new Map((stable?.artifacts ?? [])
    .filter(artifact => artifact.kind === 'standalone')
    .map(artifact => [artifact.target, artifactManifestUrl(stable, artifact)]));
  const cases = TARGETS.map(target => `  ${target}) manifest_url=${shellLiteral(urls.get(target) ?? '')} ;;`).join('\n');

  return `#!/bin/sh
set -eu
umask 077

case "$(uname -s)" in
  Linux) ;;
  *) echo "HiRoute headless installation currently supports Linux. For macOS, use HiRoute Desktop: https://hiroute.ai/en/download/" >&2; exit 1 ;;
esac

case "$(uname -m)" in
  x86_64|amd64) target=x86_64-unknown-linux-gnu ;;
  aarch64|arm64) target=aarch64-unknown-linux-gnu ;;
  *) echo "Unsupported Linux architecture: $(uname -m)" >&2; exit 1 ;;
esac

case "$target" in
${cases}
esac

if [ -z "$manifest_url" ]; then
  echo "No stable HiRoute package is published for $target yet." >&2
  exit 1
fi
command -v curl >/dev/null 2>&1 || { echo "curl is required." >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "Python 3 is required." >&2; exit 1; }

temporary=$(mktemp -d "\${TMPDIR:-/tmp}/hiroute-install.XXXXXX")
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
curl -fsSL https://hiroute.ai/install/standalone.py -o "$temporary/standalone.py"
python3 "$temporary/standalone.py" install --manifest-url "$manifest_url" "$@"

printf '%s\n' \
  "HiRoute is installed for the current user. No service was started automatically." \
  "Ensure $HOME/.local/bin is on PATH, then run:" \
  "  hiroute service start --output json" \
  "  hiroute system status --output json" \
  "Configure the console during installation with --console-mode/--console-listen/--console-port, or run hiroute console init before starting the service."
`;
}
