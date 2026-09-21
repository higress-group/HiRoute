# HiRoute

[简体中文](README.zh-CN.md)

HiRoute is a local model-routing and agent-coordination engine for long-running tasks. It
keeps model sources, routing plans, agent connections and execution evidence in one place,
with a Desktop application, a headless CLI and a local daemon.

During a long task, HiRoute can reconsider the model at Agent-turn and context-hold
boundaries. These are natural prefix changes, so routing decisions preserve the stable
prefix within each stage and remain friendly to provider KV caches. The next decision can
combine the new request with the previous stage's observed competence.

## Capabilities

- Connect supported local subscriptions, registered APIs, and compatible custom APIs.
- Publish fixed-model, smart-saving, free-first, and ordered fallback routing plans.
- Configure model access and collaboration plans for supported Agent clients.
- Run and manage Worker tasks from the CLI, with task state and results visible in Desktop.
- Inspect local sessions, request execution, known usage and cost, and delete selected data.
- Extend model selection through the public Decision API. The official Jev decider is a
  deployable reference implementation.

HiRoute is currently an MVP. The download page describes the platforms and architectures
validated for each release; unsupported or untested environments are not implied by source
availability.

## Get started

- [Download HiRoute](https://hiroute.ai/download/)
- [Documentation](https://hiroute.ai/docs/)
- [macOS self-signed installation](docs/macos-installation.md)
- [Linux headless and CLI](docs/standalone-cli.md)
- [Model-selection decisions and extensions](decision-extensions/README.md)
- [Decision API OpenAPI document](decision-extensions/api/decision.openapi.json)

For development, the Rust version is pinned by `rust-toolchain.toml`. The Desktop UI uses
Node.js/npm and Tauri. See [CONTRIBUTING.md](CONTRIBUTING.md), the
[Desktop development entry](apps/desktop/README.md), and the
[website development and release guide](apps/website/README.md).

## Repository layout

| Path | Responsibility |
| --- | --- |
| `apps/desktop` | Desktop UI and Tauri host |
| `apps/website` | Bilingual `hiroute.ai` site, downloads and OSS publication |
| `crates/cli`, `crates/daemon` | Command-line client and local service |
| `crates/client-core`, `crates/application` | Shared client and application orchestration |
| `crates/gateway-core`, `crates/gateway` | Protocol adapters, request execution and model fallback |
| `crates/observation`, `crates/local-storage` | Local execution evidence, query and persistence |
| `contracts`, `assets` | Runtime contracts, model metadata and product resources |
| `decision-extensions` | Decision mechanism, OpenAPI contract and official extensions |
| `tools`, `e2e` | Validation utilities and executable product scenarios |

## License and security

HiRoute is licensed under [Apache License 2.0](LICENSE). Report security issues and crash
reports through the process in [SECURITY.md](SECURITY.md).
