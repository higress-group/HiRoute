# HiRoute Web console

The console is served by the installed role-all `hirouted`. It uses the shared
product session and the same Application transactions as Desktop. The Gateway,
Local Control and ordinary CLI remain available independently of console assets,
listener or authentication failures. Browser pages do not connect directly to Local
Control. The external Local Control Released-command gate remains unchanged.

## Open locally

Install a complete standalone package containing `console/`, then start the service:

```sh
hiroute service start --output json
hiroute console open
```

Open the printed URL on the service host. `hiroute console open --browser` also
asks that host's browser to open it. The launch code expires after two minutes,
works once, and can only be exchanged through loopback. Do not share or log it.
These commands never implicitly start a stopped daemon.

Linux's console listener defaults to off. `open` creates a temporary loopback
listener in the running daemon; it disappears with that daemon. To keep a
loopback listener across service starts:

```sh
hiroute console enable
hiroute service restart --output json
hiroute console status
```

Automatic port selection prefers 5838 and chooses another free loopback port if
necessary. Read the actual address from `console status` or the printed launch
URL. An explicitly selected occupied port fails instead of silently changing it.
`hiroute console disable` saves the off setting for the next service start.

## Access from another device

An owner can enable a network listener and create an access token using the CLI:

```sh
hiroute console enable --listen 192.0.2.10 --port 5838 --allow-plaintext-network
hiroute service restart --output json
hiroute console token create --name workstation --expires 7 --output json
```

Replace the documentation address `192.0.2.10` with the service host's address.
Open its HTTP console URL and enter the returned token. There is no username or
password login. The token is shown only when created; the service stores its hash.
Use a separate named token for each client you want to revoke independently:

```sh
hiroute console token list --output json
hiroute console token revoke TOKEN_ID --output json
```

Revocation also ends sessions created with that token. A daemon restart ends every
browser session. Tokens are owner credentials: collaboration/Skill-authorized
callers cannot mint launch codes or tokens.

Remote HTTP is unencrypted. The page warns before access-token entry, keeps a
visible plaintext banner after login, and warns on secret-entry forms. It permits
entering model keys and other authorized secrets on that connection. Continue only
on a trusted network; no TLS or password unlock is required by the product.

An SSH tunnel is an alternative. Keep the listener on loopback and forward the
same port printed by `console open` (5838 below):

```sh
ssh -N -L 5838:127.0.0.1:5838 user@service-host
```

Use the fresh launch URL through that tunnel. The port in the browser's Host header
must match the server port; if the local port is occupied, select a different
explicit console port on the server before creating the matching tunnel.

## Sessions and confirmations

The session bearer is stored in `sessionStorage` and sent as a Bearer header.
Cookies and persistent browser storage do not carry authentication. Each tab has
its own product context. Several clients can operate concurrently; a stale edit
shows a revision conflict, refreshes actual facts and retains the unsubmitted
draft. It does not automatically merge or replay the mutation.

Up to 16 sessions stay active. A successful new login at capacity ends the least
recently used session; closing a tab alone does not sign it out. Sessions expire
after 12 hours idle or seven days total. Failed logins are limited per address
and globally, with independent loopback and network budgets.

An authenticated console session has the same protected-operation authority as
macOS Desktop. Every protected operation still presents its page confirmation and
registers authority for the exact digest. There is no password or unlock step.
Protected uses and credential lifecycle changes are audited without credentials,
secret input or conversation bodies. Closing or expiring a context releases its
protected inputs and approvals.

The browser exposes the shared model, routing, Agent, Worker, conversation and
settings workflows. Host-only update, CLI-entry, quit, login-start and titlebar
controls are absent. Directory actions show a path on the service host. OpenAPI
saving downloads the canonical file; clipboard failures expose manual copying.

Windows Desktop wraps the local console, checks the actual Gateway and creates
fresh one-time launch codes itself. A service restart returns it through the
bundled startup page and reconnects without credential entry. See
[Windows Desktop installation](windows-desktop-installation.md).

## Troubleshooting

- **Files unavailable:** reinstall the complete matching package. The service's
  API and Gateway do not depend on frontend assets being present.
- **Session ended:** create a fresh local launch URL or enter a valid access token.
  A browser does not silently acquire replacement owner credentials.
- **Too many failed sign-ins:** wait one minute before retrying. Network failures
  do not block a local launch link. Sign out before closing an unused console tab;
  a new login at the session limit retires the least recently used session.
- **Port/listener failure:** inspect the desired and applied console addresses;
  correct the saved listener and restart the service. Gateway settings are separate.
- **Origin denied:** use the actual console address and port. Requests with a
  foreign Host/Origin are refused; cross-origin management is not enabled.
- **A response was lost:** query the original Operation. Closing a browser or
  reconnecting does not cancel, retry or resubmit accepted work.
