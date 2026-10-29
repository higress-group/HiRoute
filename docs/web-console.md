# HiRoute Web console

The console is served by the installed role-all `hirouted`. It uses the shared
product session and the same Application transactions as Desktop. The Gateway,
Local Control and ordinary CLI remain available independently of console assets,
listener or authentication failures. Browser pages do not connect directly to Local
Control. The external Local Control Released-command gate remains unchanged.

## Configure during installation or initialization

A fresh unattended Linux installation keeps the console off. Choose its scope and
port explicitly when installing a complete package containing `console/`:

```sh
curl -fsSL https://hiroute.ai/install.sh | sh -s -- --console-mode loopback --console-port 5838
```

After installation, the equivalent initialization is:

```sh
hiroute console init --mode loopback --port 5838
hiroute service start --output json
hiroute console open
```

Open the printed single-use URL on the service host. `console open --browser` also
asks that host's default browser to open it. Initialization only saves settings;
service startup establishes the listener. `open` never starts a daemon or listener.
If it is off or failed to bind, inspect `console status`, configure it and restart
the service. Changes saved while running take effect at the next service start;
`open` continues returning the actual live address until then.

Omit `--port` for automatic selection: prefer 5838, then another free port. An
occupied explicit port fails without falling back. `console enable` also saves a
configuration, and `console disable` saves off for the next start. Upgrading with
no console installation options preserves existing settings byte for byte.

## Access from another device

Configure the service host's network IP and port before starting it:

```sh
hiroute console init --mode network --listen 192.0.2.10 --port 5838 --allow-plaintext-network
hiroute service restart --output json
hiroute console open
```

Replace the documentation address with the service host's actual reachable IP.
The printed URL now targets that **live network listener**. Open it on your own
computer; no desktop or browser is needed on the server. Its launch code expires
after two minutes and can be exchanged once, only at that origin and actual port.
The page removes the code from the URL before exchange. A local-only launch code
cannot be redeemed remotely. Do not share or log either kind of link.

For a wildcard bind, also configure the concrete IP to advertise in the URL:

```sh
hiroute console init --mode network --listen 0.0.0.0 --advertise-address 192.0.2.10 --port 5838 --allow-plaintext-network
```

The install-time equivalents are `--console-mode`, `--console-listen`,
`--console-advertise-address`, `--console-port` and `--allow-plaintext-network`.
They persist settings without starting the service. The advertised address must
have the bind address's IP family. DNS names and wildcard login URLs are not used.
Network reachability still depends on the host's routing and firewall.

An owner on the service host can run `hiroute console open --loopback-only` to use
its already-bound local recovery endpoint while network mode remains configured.
This selects a local one-use URL and preserves the separate local sign-in budget;
it does not create a listener or change the network configuration.

For repeated logins, an owner can alternatively create a named access token:

```sh
hiroute console token create --name workstation --expires 7 --output json
hiroute console token list --output json
hiroute console token revoke TOKEN_ID --output json
```

Open the console's network URL without a code and enter the token. There is no
username/password login. Token secrets are shown only at creation; the service
stores hashes. Revoking a token ends its derived sessions immediately. A daemon
restart ends every browser session and outstanding launch code. Collaboration or
Skill-authorized callers cannot mint launch codes or tokens.

Remote HTTP is unencrypted. The page warns before token entry, keeps a visible
plaintext banner after login (including login by link), and warns on secret-entry
forms while allowing authorized entry. Continue only on a trusted network; the
product does not require TLS or a password unlock.

An SSH tunnel is an alternative. Initialize loopback mode and forward the same
port printed by `console open` (5838 below):

```sh
ssh -N -L 5838:127.0.0.1:5838 user@service-host
```

Use the fresh local launch URL through that tunnel. Its browser Host and port
must match; if the local port is busy, configure a different server port and use
the same port for forwarding before obtaining a fresh link.

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

Versioned product commands use the same audit and confirmation flow as other
commands. Invalid operation arguments are reported as request errors and keep a
valid console session; authentication expiry and revocation still require sign-in.

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
