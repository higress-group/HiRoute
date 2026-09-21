# Contributing to HiRoute

HiRoute accepts issues and pull requests through
[higress-group/HiRoute](https://github.com/higress-group/HiRoute). Please search existing
issues before opening a new one and keep each pull request focused on one coherent change.

## Development environment

The Rust version is pinned by `rust-toolchain.toml`. The Desktop UI uses Node.js 24 and
npm; the website uses Node.js 22 and npm. Install the native packages listed in
`.github/actions/ci-setup/action.yml` when building the backend on Debian or Ubuntu.

Common checks are:

```sh
cargo fmt --check
cargo clippy --locked --workspace --exclude hiroute-desktop --all-targets --all-features -- -D warnings
cargo test --locked --workspace --exclude hiroute-desktop --all-features

cd apps/desktop
npm ci --ignore-scripts
npm run build
npm test

cd ../website
npm ci --ignore-scripts
npm test
npm run build
```

Run the checks relevant to your change before opening a pull request. Native macOS Desktop
behavior and real-provider integration require their respective environments; describe any
such checks that you could not run.

## Language boundaries

English is the default language for source comments, GitHub metadata, and Markdown files
without a language suffix. Put a Simplified Chinese translation in a sibling
`*.zh-CN.md` file and keep links within the reader's current language whenever a translated
page exists. Label each language switch in the target language: use `简体中文` from English
and `English` from Chinese. Website guide sources use the existing `*.en.md` and `*.zh.md`
convention.

Desktop and website components may contain both locales in source, but every user-visible
sentence must be selected by the active locale; do not leave English decorative copy on a
Chinese page or Chinese fallback copy on an English page. Product names, protocol names,
commands, paths, code, API fields, and model/provider identifiers remain unchanged. Synthetic
multilingual fixtures are allowed when they test Unicode or localized behavior.

Internal issue specifications, handoffs, and workflow instructions are outside this
public-language boundary.

Do not commit API keys, account data, real conversation content, diagnostic archives, or
machine-specific configuration. Use synthetic fixtures in tests. Security issues and crash
reports follow [SECURITY.md](SECURITY.md).

By contributing, you agree that your contribution is licensed under Apache License 2.0.
