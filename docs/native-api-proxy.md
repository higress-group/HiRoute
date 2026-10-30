# Native API network settings

Each saved Native API connection owns one network policy. Its ordinary models,
protocol endpoints and decision models that explicitly share the connection use
that policy. A Worker request routed through Gateway uses the selected connection's
policy. Subscription proxy settings and Worker process/tool environments have their
own owners.

New and upgraded Native connections use **Direct**. Open a connection's optional
**Network and proxy** section to choose another mode. Details start collapsed for
new connections and ordinary reopening, including an existing manual policy. The
compact summary shows the mode and route/application state. Expanding or collapsing
preserves inputs and never checks a provider.

| Mode | Behavior |
| --- | --- |
| Direct | Ignore proxy environment for this connection. |
| Service environment | Use the proxy environment captured when the HiRoute service started. Restart the service after changing that environment. |
| Manual | Use one HTTP(S) proxy URL and optional bypass list for this connection. |

Manual URLs accept a host and port with an optional `/`; credentials, other paths,
query and fragment are rejected. For a proxy requiring Basic authentication,
configure its URL credentials in the service startup environment and select the
environment mode. Protected inherited credentials are never returned in status or
errors. Proxy authorization and the provider API Key have separate destinations.
In a remote console, `127.0.0.1` refers to the service machine.

## Environment and bypass rules

The nonempty uppercase variable takes precedence over its lowercase equivalent.
An HTTP destination uses `HTTP_PROXY`/`http_proxy`; an HTTPS destination uses
`HTTPS_PROXY`/`https_proxy`. They do not substitute for one another. An absent
selected proxy means Direct with a missing-proxy explanation. An invalid selected
proxy or effective bypass list means Unavailable, with no direct retry. `ALL_PROXY`,
PAC and operating-system proxy UI are not consulted.

Manual mode uses its own bypass list; environment mode uses `NO_PROXY`/`no_proxy`.
Entries are comma-separated and surrounding whitespace is ignored.

| Entry | Match against the original destination |
| --- | --- |
| `example.test`, `.example.test` | Exact host and dot-boundary subdomains, case-insensitive and canonicalized. |
| `example.test:8443` | That host rule at the effective port. |
| IP literal, optionally a port | Exact address; bracket IPv6 when specifying a port. |
| IPv4/IPv6 CIDR | Original IP-literal destinations only; no DNS lookup for matching. |
| `*` | Every destination. |

Empty entries are ignored; malformed entries are rejected. `localhost`, its
subdomains, `127.0.0.0/8` and `::1` always bypass. Other private addresses do not
implicitly bypass. A loopback proxy remains a valid next hop for a non-loopback API.
A shared decision URL is matched independently of the ordinary inference URL.

## Save, check and repair

Saving a policy affects subsequent HTTP requests immediately without restarting
or republishing a Plan. An admitted request, including its decision and model
execution legs, retains one network snapshot. A running stream keeps its route.
Connections from an incompatible snapshot are not reused for newer work.

An existing connection's combined save applies checked connection edits and its
policy atomically, with source and network revision guards. A policy change shows
its affected ordinary models and retained shared decision configurations before
Apply. Default Direct creation adds no network confirmation step. Network-only
repair can be saved without a successful remote check.

“Applied” means the runtime admitted that configuration, not that a provider is
reachable. Directory results, provider authentication and an explicit model/tool
test remain separate evidence. No directory preserves the declared-model path
without claiming connectivity. Policy or captured-environment changes invalidate
an earlier successful test; checking again is explicit.

Saved recheck, append, trial reuse and shared decision settings offer the same
source-owned network editor. It preserves the parent model/URL/codec/selection or
trial request. Its successful save is independent of the parent: cancelling the
parent does not undo the source change. A pending or uncertain save follows its
original Operation rather than resubmitting. A route-only edit retains a
same-context protected Key while invalidating the earlier check's authority.

Typed failures distinguish proxy connection, proxy TLS/authentication/negotiation,
upstream failures, directory/inference failure and an unknown transport stage.
A proxy 407 offers network repair and does not disable or request replacement of
the provider Key. An ambiguous failure retains neutral wording. A proxy failure
never silently retries the same destination directly; permitted Plan fallback
still obeys its existing order, budgets and response-commit boundary.

## CLI and shared hosts

Read the installed help/schema and retain the complete machine envelope privately.
The following operations also back Desktop and the authenticated console:

| Command | Input and result |
| --- | --- |
| `compute network show` | `source_id`; desired policy, applied state and affected consumers. |
| `compute network resolve` | A draft, registered, saved-source or exact saved-decision target and optional policy override; safe route preview without DNS or provider calls. |
| `compute network preview` | Source revision plus network revision/policy; sealed preview and before/after views. |
| `compute network apply` | Exact preview spec, digest and revisions plus one retained idempotency key. |
| `compute connection test-network` | Strict Native/registered/saved V2 wrapper; `check`, admitted `network` and typed `failure`. New drafts use network revision 0. |
| `compute connection preview-network` | Strict V4 carrier wrapping the existing connection change and explicit network edit. |
| `decision network test` | Exact saved ID, revision and configuration digest; V3 result with `check`, admitted `network` and typed `failure`. |

Use `--request-file` or `--request-stdin` for these typed inputs. For example,
`compute network resolve` accepts:

```json
{
  "target": { "kind": "decision", "id": "decision/example", "revision": 1 },
  "policy": null
}
```

An explicit decision test may use quota. Inspect `data.check.outcome` even after
command success. Its network identity is independent of credential state and
reflects the admitted call. Independent/custom decisions have no inherited Native
network evidence. The released `decision connections test` retains its strict V2
result while using the same production diagnostic.

Policy storage and recovery belong to the source's existing Control transaction;
[the durable connection contract](../issue-spec/specs/model-connections/spec.md)
and [decision contract](../issue-spec/specs/decision-services/spec.md) own the
requirements. Transport implementation starts at
[Gateway](../crates/gateway/README.md) and the [code map](code-map/architecture.md).
