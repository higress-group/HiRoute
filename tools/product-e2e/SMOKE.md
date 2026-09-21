# Current-candidate production smoke

[简体中文](SMOKE.zh-CN.md)

`hiroute-smoke` is a development-validation entry point. Its default quick run has three
required cases: a controlled Responses request through the production Gateway; real
CLI/control-role daemon status plus read-only `client_access`, idempotent lookup, and
controlled discovery through the shared Client Core; and safe rejection of tampered
current-directory resources. These cases do not prove successful control-plane publication,
a real Agent/account, Desktop, or installation.

## Run

Follow the repository's remote Rust convention: commit and push an isolated candidate, then
run:

```sh
python3 scripts/remote-rust.py run --ref refs/heads/<branch> --sha <full-commit> -- \
  cargo test --locked -p hiroute-product-e2e --test smoke_cli -- --nocapture
```

The test invokes the development tool and checks fresh results for all three default cases,
the control domain, and one Gateway case. A remote `completed` result proves only Cargo exit
and non-zero Rust test selection. Also inspect every case result and
`target/smoke/<run-id>/report.json` in the checkout.

The default `smoke_cli` executes the three real paths once through
`default_two_domain_smoke`, including their steps, scopes, outcomes, and report. The
non-starting `validate` tests cover CLI domain/case selectors. Do not duplicate the default
run with all three individual cases; rerun an exact case only for diagnosis.

`target/smoke/run.lock` protects real production subprocesses and recovery materials in one
checkout. The build-proof process is unchanged; do not inject an unverified older binary to
shorten it.

After the tool is built in the validation checkout, direct selectors are available:

```sh
target/debug/hiroute-smoke list
target/debug/hiroute-smoke validate --domain control
target/debug/hiroute-smoke run
target/debug/hiroute-smoke run --domain control
target/debug/hiroute-smoke run --case gateway.responses.controlled
```

`list/validate` checks a catalog or selection without running the product. Domain and case
are exact IDs; their combination is an intersection. Unknown, duplicate, zero-selection, or
missing requested cases fail. To rerun an old report's `expected_cases`, pass each ID as a
`--case`; every ID is resolved against the current registry with new identities and private
resources. Old successful results and configuration are never loaded.

## Results and recovery

The report records tool exit, product-step exit/signal, per-case
green/expected_red/red, reasons for non-execution, source/artifact identity, build and
execution timing, cleanup state, and capability gaps. Large debug-artifact integrity checks
run before and after each product command and do not consume the 30-second product-scenario
or 10-second product-step budgets. Safe rejection of a tampered current-directory resource
can be a green negative case even when the daemon exits non-zero. Undelivered Agent,
real-account, Desktop, or installation adapters must be `not_executed`, never `expected_red`.
`Report::verify` checks structural consistency and cannot authenticate foreign JSON as
production evidence.

Subprocesses use only the current isolated directory. The control discovery script uses
synthetic input and never calls an everyday Codex/Claude account. A production Agent adapter
is not connected here; existing component contracts require all configuration layers to be
isolated, formally restored, and checked before cleanup. Preserve recovery context after a
conflict or failed restore instead of copying a backup over user configuration.

Logs and recovery locators remain private and are not uploaded automatically. Normal
cancellation stops new work and reclaims this run's processes. SIGKILL, host restart, or a
remote hard deadline may leave interrupted resources. If `private/run.journal` shows an
unfinished run, inspect that PID and its resources manually; never turn an old journal into
green evidence or bulk-delete active work. Failed runtimes retain a short private temporary
directory; successful ones clean it. Cargo always uses this checkout's default `target/`.

## Domain contributions

A domain registers a bounded case in `src/smoke/registry.rs` and owns its real action and
business assertions. The central runner owns only selection, lifecycle, and the common
report. Domain 17 consumes the read-only CLI → shared Client Core → Application
`client_access` and `FindOperationByIdempotency` paths in the control case. Domain 01 owns
control-plane publication/recovery semantics; Domain 02 owns real Desktop windows, native
confirmation, and rename scenarios; Agent and installation domains own formal recovery and
platform evidence. Do not replace a claimed product entry with an internal Application seam,
handwritten socket, or fixture output.

If an inner Gateway failure occurs before timing decomposition is returned, the report keeps
known `build_ms`, assigns the remaining wall time to `unclassified_ms`, sets
`timing_complete=false`, and never presents `execution_ms` as pure product time.
