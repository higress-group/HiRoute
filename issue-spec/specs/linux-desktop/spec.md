# Linux Desktop

## Purpose

Use HiRoute from the Omarchy launcher and manage the same current-user runtime
used by the installed CLI. Linux Desktop owns its window and native effects.
The standalone service owns product state, request execution and the console.

## Requirements

### Requirement: Install the desktop and runtime as separate operations

The architecture-specific Arch package SHALL provide a single HiRoute entry,
the native executable and a verified standalone bundle from the same source.
The package transaction SHALL scope its changes to package-owned files. A
current-user first-run action SHALL verify the bundled runtime before creating
its CLI, user unit or loopback listener. Login startup is opt-in. Existing
listener settings and ownership remain authoritative when an installation is
found.

#### Scenario: Install the package and cancel first run

- **WHEN** A user installs the Arch package, opens HiRoute from Apps and closes
  the setup window before confirming
- **THEN** HiRoute retains its launcher entry and creates no standalone install
  record, user unit or listener as a result of that setup attempt.

### Requirement: Qualify the served console and native window session

Linux Desktop SHALL verify the user manager and running executable, negotiate
the supported service state and qualify the exact console files served by that
process. A short-lived loopback launch SHALL authenticate each window session.
Native effects SHALL require the admitted window and its live launch session.
A verified healthy standalone release that is ineligible for native attachment
can supply a separate one-use browser launch through its own installed CLI.

#### Scenario: Reopen from Apps while a window is already open

- **WHEN** A user activates the HiRoute entry with the original window open
- **THEN** The compositor focuses the original Wayland window using the same
  desktop identity. Closing that window leaves the current-user service available.

### Requirement: Share presentation and expose host-specific effects

Linux Desktop SHALL use the product console, semantic styles and native
compositor framing. A tiled window SHALL keep the main actions keyboard
reachable. Follow system uses the desktop session's standard appearance signal.
Explicit light/dark and language preferences SHALL persist across processes
and loopback origins. Presentation settings SHALL have a typed per-user record
separate from session credentials. Linux host commands SHALL expose the typed
capabilities implemented by its adapter.

#### Scenario: Change the system theme with HiRoute open

- **WHEN** The user applies a light or dark system theme with HiRoute set to
  Follow system
- **THEN** The open window follows the system. An explicit HiRoute choice
  takes precedence and persists on reopening.

### Requirement: Upgrade and remove the user runtime explicitly

Replacing or removing the package SHALL leave the current-user runtime running.
A native upgrade SHALL offer the verified, newer package payload only through
an explicit preview and confirmation. Preview reports the versions and restart
impact, verifies the package and runs storage preflight. Apply SHALL recheck
the approved payload and live owner under the install lock before stopping the
user service. It SHALL preserve the listener, startup settings and routing
data through the standalone install and recovery paths.

The native removal action SHALL require a live admitted session and confirmation.
The installer SHALL verify runtime ownership and check managed Agent settings
before stopping the service and removing owned components. Outstanding Agent
restores use the ordinary Agent workflow. The operation SHALL preserve saved
routing data, history, credentials and the installed desktop package.

#### Scenario: Defer an available runtime update

- **WHEN** A user cancels the update confirmation
- **THEN** The previous service keeps handling requests and can open a new
  authenticated browser session.

#### Scenario: Managed Agent settings require restoration

- **WHEN** The user confirms removal with outstanding managed settings
- **THEN** The native console directs the user to the Agent restore workflow
  and the service and install remain available.

### Requirement: Recover interrupted user runtime publication

The standalone installer SHALL durably record publication before changing
managed program, resource, unit, CLI entry and marker paths. A current runtime
startup SHALL refuse an unfinished publication. Recovery SHALL validate the
recorded ownership and complete the committed generation or restore the
prepared generation's previous installation. Storage admission separately
governs writer changes and retained business state.

#### Scenario: Interrupt publication after changing the user unit

- **WHEN** The standalone installer is interrupted after replacing the user
  unit and before committing publication
- **THEN** The current CLI and daemon refuse startup through the incomplete
  installation. Installer recovery restores the previous program and unit
  together, preserving its routing and session data.
