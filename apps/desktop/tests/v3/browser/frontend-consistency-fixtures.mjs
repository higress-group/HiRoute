// Read-only, deterministic fixture data for the actual Settings components.
// These replies do not establish native service, upgrade or filesystem effects.
export function consistencySettingsReads(control) {
  control.handlers.update_status = () => ({ current_version: '0.5.0', phase: 'idle', available: null,
    downloaded_bytes: 0, active_calls: 0, active_tasks: 0, error: null, can_install: true,
    package_ready: false, recovery: null });
  control.handlers.cli_entry_status = () => ({ state: 'valid', entry_path: '/opt/hiroute/bin/hiroute',
    path_contains_entry_directory: true, command_resolves_to_app_cli: true, daemon_available: true });
  control.handlers.gateway_listener_status = () => ({ config: {
    schema_version: 'hiroute.gateway-listener/v1', desired: { address: '127.0.0.1', port_mode: 'automatic' },
    applied: { address: '127.0.0.1', port: 5845, applied_at_unix: 1_793_404_800 },
  }, connect_address: 'http://127.0.0.1:5845', existing_agent_count: 0, ready: true });
  control.handlers.diagnostic_status = () => ({ schema: 'hiroute.diagnostics-status/v1',
    settings: { revision: 1, level: 'info' }, logs_directory_available: true });
}
