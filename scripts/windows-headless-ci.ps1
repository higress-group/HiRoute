param(
  [ValidateSet('components','product','background','core','agents','accounts','upgrade','install')][string]$Scope,
  [Parameter(Mandatory)][string]$ExpectedSha,
  [Parameter(Mandatory)][string]$Evidence,
  [string]$Plan = 'windows-headless',
  [ValidateSet('focused','final')][string]$Phase = 'focused',
  [string[]]$RelatedRun = @(),
  [string]$CpaEvidence,
  [string]$CpaSource,
  [string]$CpaOriginalRun,
  [string]$CpaOriginalRevision,
  [string]$BaselineManifest,
  [string]$BaselineArchive,
  [string]$BaselineRevision,
  [switch]$CaptureArtifacts,
  [switch]$InventoryOnly,
  [ValidateSet('full','hosted')][string]$Assessment='full'
)
$ErrorActionPreference = 'Stop'
$evidence = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Evidence)
$failures = [System.Collections.Generic.List[string]]::new()
$scopeExceptionFailed = $false
$artifactVerified = $false
$importsVerified = $false
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
. "$PSScriptRoot/windows-native-scopes.ps1"
. "$PSScriptRoot/windows-upgrade-inputs.ps1"
if ($Assessment -eq 'hosted' -and ($Scope -cne 'background' -or -not $CaptureArtifacts -or $InventoryOnly)) {
  throw 'Hosted assessment requires complete background artifact capture'
}
if ($InventoryOnly -and ($Scope -notin @('components','background','agents','accounts') -or $CaptureArtifacts)) {
  throw 'Inventory export requires a static native scope and no artifact capture'
}
$inventoryRecipes = [System.Collections.Generic.List[object]]::new()
if (-not $IsWindows) { throw 'Native Windows required' }
if ($ExpectedSha -notmatch '^[0-9a-f]{40}$' -or (git rev-parse HEAD) -ne $ExpectedSha) {
  throw 'Exact committed candidate required'
}
git diff --exit-code HEAD --
if ($LASTEXITCODE -ne 0) { throw 'Candidate has tracked changes' }
if ($env:CARGO_TARGET_DIR -or $env:CARGO_BUILD_TARGET_DIR) { throw 'Checkout-local Cargo target required' }
New-Item -ItemType Directory -Force $evidence | Out-Null
function Initialize-WindowsNativeBuildContext {
$contextScript = @'
import importlib.util, json, os
from pathlib import Path
spec = importlib.util.spec_from_file_location('native_release_context', Path('scripts/build-release.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
# CRT flags are architecture-independent; the native rustc host selects the target.
context = module.windows_release_environment('x86_64-pc-windows-msvc', dict(os.environ))
print(json.dumps({key: context[key] for key in ('CARGO_ENCODED_RUSTFLAGS', 'CMAKE_TOOLCHAIN_FILE')}))
'@
$contextJson = python -c $contextScript
if ($LASTEXITCODE -ne 0) { throw 'Canonical Windows static CRT compile context failed' }
$context = $contextJson | ConvertFrom-Json
if (@($context.PSObject.Properties.Name).Count -ne 2 -or
    $context.CARGO_ENCODED_RUSTFLAGS -cne ("-C" + [char]31 + "target-feature=+crt-static") -or
    -not $context.CMAKE_TOOLCHAIN_FILE) { throw 'Canonical Windows compile context differs' }
$env:CARGO_ENCODED_RUSTFLAGS = $context.CARGO_ENCODED_RUSTFLAGS
$env:CMAKE_TOOLCHAIN_FILE = $context.CMAKE_TOOLCHAIN_FILE
}
if (-not $InventoryOnly) {
Initialize-WindowsNativeBuildContext
$env:HIROUTE_BUILD_SOURCE_SHA = $ExpectedSha
$env:CARGO_TERM_COLOR = 'never'
$env:CARGO_INCREMENTAL = '0'
$os = Get-CimInstance Win32_OperatingSystem
@{ source_revision = $ExpectedSha; validation_revision = $env:GITHUB_SHA; scope = $Scope;
   assessment=$Assessment; runtime_recipes=@(Get-WindowsAssessmentNames $Scope $Assessment 'execute');
   deferred_recipes=@(Get-WindowsAssessmentNames $Scope $Assessment 'compile-only');
   full_scope=($Assessment -eq 'full'); candidate_complete=$false;
   plan = $Plan; phase = $Phase; related_runs = $RelatedRun; caption = $os.Caption;
   version = $os.Version; cpu_count = [Environment]::ProcessorCount;
   host_architecture = [Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString();
   image_version = $env:ImageVersion; rustc = (rustc --version); cargo = (cargo --version)
} | ConvertTo-Json -Depth 5 | Out-File -Encoding utf8 "$evidence/provenance.json"
}

function Get-NativeTestFilter {
  param([string[]]$CargoArguments)
  $filter = $null
  for ($i = 0; $i -lt $CargoArguments.Count; $i++) {
    if ($CargoArguments[$i] -in @('-p','--package','--test','--bin','--features')) { $i++; continue }
    if (-not $CargoArguments[$i].StartsWith('-')) {
      if ($filter) { throw 'Multiple native test filters are not supported' }
      $filter = $CargoArguments[$i]
    }
  }
  return $filter
}

function Invoke-RequiredTests {
  param([string]$Name, [string[]]$CargoArguments, [string[]]$Required, [string[]]$HarnessArguments = @())
  if ($InventoryOnly) {
    if ($Required.Count -eq 0 -or @($Required | Sort-Object -Unique).Count -ne $Required.Count -or
        @($inventoryRecipes | Where-Object { $_.name -ceq $Name }).Count -ne 0) { throw 'Native source inventory must be nonempty and unique' }
    $inventoryRecipes.Add(@{ name=$Name; required_cases=$Required; cargo_arguments=$CargoArguments;
      harness_arguments=$HarnessArguments; filter=(Get-NativeTestFilter $CargoArguments) })
    return
  }
  $result = @{ scenario = 'red'; name = $Name; cases = $Required; executed = 0;
               source_revision = $ExpectedSha; ignored = 0; process_exit = $null;
               plan = $Plan; phase = $Phase; related_runs = $RelatedRun;
               command = @('cargo','test','--locked') + $CargoArguments + @('--','--test-threads=1') + $HarnessArguments }
  $disposition=Get-WindowsRuntimeDisposition -Scope $Scope -Name $Name -Assessment $Assessment
  $result.assessment=$Assessment
  $result.runtime_executed=$false
  $result.artifact_capture_verified=$false
  $result.build_status='not_assessed'
  $result.inventory_verified=$false
  $result.inventory_exit=$null
  $started = Get-Date
  try {
    Initialize-WindowsQoderBashFixture -Name $Name
    cargo test --locked @CargoArguments -- --list 2>&1 | Tee-Object "$evidence/$Name-inventory.log"
    $result.inventory_exit=$LASTEXITCODE
    if ($LASTEXITCODE -ne 0) { throw "$Name inventory failed to build" }
    $inventory = Get-Content "$evidence/$Name-inventory.log" -Raw
    foreach ($case in $Required) {
      $escaped = [regex]::Escape($case)
      if ($inventory -notmatch "(?m)^${escaped}: test`r?$") { throw "Required case not selected: $case" }
    }
    $result.build_status='compiled'
    $result.inventory_verified=$true
    if ($CaptureArtifacts) {
      # Cargo's progress belongs to stderr even with --message-format=json.
      # Keep the compiler-artifact stream parseable and retain diagnostics separately.
      cargo test --locked @CargoArguments --no-run --message-format=json > "$evidence/$Name-cargo-artifacts.log" 2> "$evidence/$Name-cargo-artifacts-stderr.log"
      if ($LASTEXITCODE -ne 0) {
        Get-Content "$evidence/$Name-cargo-artifacts-stderr.log" | Write-Host
        throw "$Name executable provenance failed to build"
      }
      $filter = Get-NativeTestFilter $CargoArguments
      @{ required_cases = $Required; cargo_arguments = $CargoArguments; harness_arguments = $HarnessArguments; filter = $filter } |
        ConvertTo-Json -Depth 5 | Out-File -Encoding utf8 "$evidence/$Name-recipe.json"
      python scripts/windows-native-artifacts.py capture --expected-sha $ExpectedSha --evidence $evidence --name $Name --cargo-json "$evidence/$Name-cargo-artifacts.log" --recipe-json "$evidence/$Name-recipe.json"
      if ($LASTEXITCODE -ne 0) { throw "$Name executable provenance failed" }
      $result.artifact_capture_verified=$true
    }
    if ($disposition -eq 'compile-only') {
      if (-not $CaptureArtifacts -or -not $result.artifact_capture_verified) { throw 'Verified deferred executable capture required' }
      Assert-WindowsDeferredInventory -Inventory $inventory -Required $Required
      $result.command=@('cargo','test','--locked')+$CargoArguments+@('--no-run','--message-format=json')
      $result.scenario='not_assessed'
      $result.build_status='compiled'
      $result.inventory_verified=$true
      $result.deferred_reason='requires_real_logged_owner'
      return
    }
    $result.command_started_at = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() / 1000.0
    $result.runtime_executed=$true
    cargo test --locked @CargoArguments -- --test-threads=1 @HarnessArguments 2>&1 | Tee-Object "$evidence/$Name.log"
    $result.process_exit = $LASTEXITCODE
    $result.command_finished_at = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() / 1000.0
    $log = Get-Content "$evidence/$Name.log" -Raw
    foreach ($case in $Required) {
      $escaped = [regex]::Escape($case)
      if ($log -match "(?m)^test ${escaped} \.\.\. (ok|FAILED)`r?$") { $result.executed++ }
    }
    $summaries = [regex]::Matches($log, '(?m)^test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored.*$')
    if ($summaries.Count -gt 0) {
      # --nocapture can interleave per-case output; use the terminal parent summary.
      $summary = $summaries[$summaries.Count - 1]
      $result.tests = @{ passed = [int]$summary.Groups[1].Value; failed = [int]$summary.Groups[2].Value; ignored = [int]$summary.Groups[3].Value }
      $result.executed = $result.tests.passed + $result.tests.failed
      $result.ignored = $result.tests.ignored
    }
    if ($result.process_exit -ne 0) { throw "$Name native acceptance failed" }
    if ($log -notmatch "test result: ok\. $($Required.Count) passed; 0 failed; 0 ignored") {
      throw "$Name required runtime count differs"
    }
    $result.executed = $Required.Count
    $result.scenario = 'green'
  } catch {
    $failures.Add($_.Exception.Message)
    Write-Warning $_.Exception.Message
  } finally {
    $result.seconds = ((Get-Date) - $started).TotalSeconds
    $result | ConvertTo-Json -Depth 5 | Out-File -Encoding utf8 "$evidence/$Name-result.json"
  }
}

function Invoke-LibraryTests {
  param([string]$Package)
  $name = $Package -replace '^hiroute-','core-'
  try {
    cargo test --locked -p $Package --lib -- --list 2>&1 | Tee-Object "$evidence/$name-discovery.log"
    if ($LASTEXITCODE -ne 0) { throw "$Package native library inventory failed" }
    $inventory = Get-Content "$evidence/$name-discovery.log" -Raw
    $optIn = @()
    # These source-declared opt-ins need Release timing or a private stopped
    # history. Ordinary core execution must not count them as ignored passes.
    if ($Package -eq 'hiroute-application') {
      $optIn = @('observation_query::control::trust_tests::trust_suggestions_cached_read_cost')
    }
    if ($Package -eq 'hiroute-observation') {
      $optIn = @(
        'query_v2::session_search_tests::live_search_250k_occurrences_and_60k_sources_under_real_deadline',
        'query_v2::session_search_tests::live_search_450k_occurrences_and_100k_sources_under_real_deadline',
        'store::field_replay_tests::captured_native_suffix_uses_the_real_writer',
        'store::field_tests::current_history_and_real_maintenance',
        'store::tests::plan_quality::routing_timeline::timeline_long_session_read_cost',
        'store::tests::plan_quality::trust::trust_suggestion_population_read_cost')
    }
    if ($Package -eq 'hiroute-gateway') {
      $optIn = @('server::core_runtime::adapters::tests::local_work::benchmark_local_text_processing')
    }
    if ($Package -eq 'hiroute-integrations') {
      $optIn = @(
        'agents::native_ingress_probe::ephemeral::tests::real_codex_ephemeral_authentication_without_managed_configuration',
        'agents::qoder_probe::tests::real_qoder_discovers_skill_and_runs_read_only_cli_in_selected_context',
        'agents::qoder_probe::windows_cli::tests::windows_qoder_bash_executes_the_same_cli_with_literal_drive_path',
        'agents::windows_runtime::tests::windows_native_environment_fixture',
        'agents::windows_runtime::tests::windows_selected_node_version_uses_the_private_capture_and_held_job',
        'agents::windows_runtime::tests::windows_selected_pi_sdk_capabilities_load_the_actual_package',
        'agents::windows_collaboration_tests::actual_windows_codex_native_skill_executes_the_public_cli',
        'agents::windows_collaboration_tests::actual_windows_claude_native_skill_executes_the_public_cli',
        'agents::windows_collaboration_tests::actual_windows_qoder_native_skill_executes_the_public_cli_without_synthesized_account')
    }
    if ($Package -eq 'hiroute-cpa-bridge') {
      $optIn = @(
        'accounts::stock::pin_tests::live_managed_cpa_pin_reads',
        'runtime::tests::borrowed_stock::stock_cpa_v7_2_140_accepts_managed_auth_ready_and_shutdown_contract',
        'runtime::tests::borrowed_stock::stock_cpa_v7_2_140_real_borrowed_codex_responses_smoke')
      foreach ($case in @(
        'windows_private::tests::private_publication_and_owner_claim_use_actual_native_owner_and_acl',
        'windows_private::tests::private_io_rejects_foreign_acl_hardlink_and_junction_without_adoption',
        'windows_private::tests::generated_auth_files_keep_user_owner_and_inherited_private_acl',
        'windows_private::tests::damaged_private_record_is_preserved_and_never_rebuilt',
        'borrowed_codex::windows_tests::windows_borrowed_native_rejects_same_size_time_parent_replacement_before_read',
        'borrowed_codex::windows_tests::windows_borrowed_native_rejects_ancestor_junction_and_hardlinks',
        'borrowed_codex::windows_tests::windows_borrowed_native_preserves_readable_acl_readonly_and_access_only_lease',
        'borrowed_codex::windows_tests::windows_borrowed_native_permission_change_rejects_before_lease_publication',
        'borrowed_codex::windows_tests::windows_borrowed_native_rejects_post_write_change_before_identity_admission')) {
        if ($inventory -notmatch "(?m)^$([regex]::Escape($case)): test`r?$") { throw "Required CPA private case missing: $case" }
      }
    }
    if ($Package -eq 'hiroute-cli') {
      foreach ($case in @(
        'service::windows::logs::tests::windows_service_logs_reads_actual_daemon_writer_and_bounds_rotated_tails',
        'service::windows::logs::tests::windows_service_logs_refuses_hardlinks_and_junctions_before_reading_external_bytes',
        'service::windows::logs::tests::windows_service_logs_keeps_held_root_identity_and_does_not_create_missing_roles')) {
        if ($inventory -notmatch "(?m)^$([regex]::Escape($case)): test`r?$") { throw "Required Windows logs case missing: $case" }
      }
    }
    $required = @([regex]::Matches($inventory, '(?m)^(\S+): test\r?$') | ForEach-Object { $_.Groups[1].Value } | Where-Object { $_ -notin $optIn })
    if ($required.Count -eq 0) { throw "$Package selected no native library tests" }
    $harness = @($optIn | ForEach-Object { '--skip'; $_ })
    @{ source_revision = $ExpectedSha; package = $Package; explicit_opt_in_or_child_fixtures = $optIn;
       reason = 'Real selected installation inputs and helper children have separate explicit native gates; any unregistered ignored case still fails this inventory.'
    } | ConvertTo-Json -Depth 5 | Out-File -Encoding utf8 "$evidence/$name-selection.json"
    Invoke-RequiredTests $name @('-p',$Package,'--lib') $required $harness
  } catch { $failures.Add($_.Exception.Message); Write-Warning $_.Exception.Message }
}

function Initialize-WindowsAgentInputs {
  if ($InventoryOnly) { return }
  cargo build --locked -p hiroute-daemon --bin hirouted -p hiroute-cli --bin hiroute 2>&1 | Tee-Object "$evidence/agent-daemon-link.log"
  if ($LASTEXITCODE -ne 0) { throw 'Production Worker daemon failed to build' }
  python scripts/windows-agent-fixtures.py --daemon target/debug/hirouted.exe --output "$evidence/clients" > "$evidence/clients-result.json"
  if ($LASTEXITCODE -ne 0) { throw 'Fixed upstream client prerequisites failed; Agent scenarios not executed' }
  $clients = Get-Content "$evidence/clients-result.json" -Raw | ConvertFrom-Json
  foreach ($property in $clients.environment.PSObject.Properties) {
    if ($property.Name -notmatch '^HIROUTE_WORKER_[A-Z_]+$') { throw 'Unknown client environment key' }
    [Environment]::SetEnvironmentVariable($property.Name, [string]$property.Value, 'Process')
  }
  $env:PATH = (@($clients.discovery_path) + @($env:PATH)) -join [IO.Path]::PathSeparator
  $env:HIROUTE_NATIVE_CODEX = $env:HIROUTE_WORKER_CODEX_BINARY
}

try {
if ($Scope -eq 'components') {
  Invoke-RequiredTests 'platform' @('-p','hiroute-platform','--test','windows') @(
    'private_file_identity_is_held_and_reparse_free',
    'ancestor_junction_cannot_redirect_private_read_or_creation',
    'pipe_exclusivity_bidirectional_frames_and_peer_identity',
    'pipe_deadline_cancels_read_without_consuming_the_next_frame',
    'native_permissions_survive_atomic_restore_and_detect_acl_changes',
    'sealed_readonly_artifact_rename_preserves_bytes_identity_and_native_permissions',
    'readonly_atomic_restore_preserves_permissions_and_requires_target_write_attributes')
  Invoke-RequiredTests 'read-leases' @('-p','hiroute-platform','--test','windows_read_lease') @(
    'read_lease_preserves_borrowed_acl_and_pins_file_and_all_ancestors',
    'read_lease_rejects_aliases_without_mutating_their_targets')
  Invoke-RequiredTests 'platform-unit' @('-p','hiroute-platform','--lib') @(
    'durable::tests::initial_private_publication_never_replaces_a_committed_key',
    'local_record::tests::rejects_wrong_channel_direction_truncation_end_and_oversize',
    'private_fs::tests::owned_cleanup_is_idempotent_and_preserves_neighbor_materials',
    'private_fs::tests::private_materials_are_exclusive_and_read_without_permission_adoption',
    'private_fs::tests::storage_preparation_reopens_without_adopting_a_file',
    'private_fs::tests::windows_owned_root_stays_pinned_and_private_read_does_not_repair_borrowed_acl',
    'windows::background::tests::native_argv_quotes_spaces_quotes_and_trailing_slashes_without_shell',
    'windows::background::tests::profile_is_absolute_and_definition_rejects_unknown_or_foreign_owner',
    'windows::lifecycle::tests::lifecycle_payload_is_closed_and_does_not_accept_extra_authority',
    'windows::service::token::tests::restricted_business_child_runs_with_owner_medium_token',
    'windows::task::tests::registration_preserves_demand_start_and_logon_policy_across_reopen') @('--nocapture')
  Invoke-RequiredTests 'jobs' @('-p','hiroute-platform','--test','job_windows') @(
    'job_stops_surviving_tool_after_leader_exit_and_preserves_neighbor',
    'native_job_argv_preserves_spaces_quotes_unicode_and_shell_characters',
    'managed_helper_executes_from_cmd_with_literal_special_character_path_and_argv',
    'duplicate_windows_environment_keys_are_rejected_before_launch',
    'inherited_credential_pipe_roundtrips_and_preserves_original_handle',
    'inherited_credential_rejects_invalid_stdio_file_and_noninherited_handles',
    'inherited_credential_pipe_timeout_and_byte_limit_are_bounded',
    'inherited_credential_crosses_an_explicit_job_child_handle_list') @('--skip','job_probe')
  Invoke-RequiredTests 'console' @('-p','hiroute-platform','--test','windows_console','gui_') @(
    'gui_request_without_a_parent_console_preserves_pipes_and_creates_no_console',
    'gui_attach_preserves_inherited_pipes_with_a_real_parent_console')
  Invoke-RequiredTests 'diagnostics' @('-p','hiroute-diagnostics','--test','windows_files_safety') @(
    'held_directory_read_write_enumerate_rename_and_remove_ignore_a_path_replacement',
    'existing_accessible_native_acl_is_preserved_and_new_objects_are_private',
    'junction_ancestor_and_child_cannot_redirect_diagnostic_operations',
    'hardlinks_and_replaced_file_identities_are_refused_before_mutation',
    'held_writer_can_atomically_replace_and_delete_without_share_mode_self_lock',
    'lock_contention_settings_commit_and_real_runtime_writer_are_supported')
  Invoke-RequiredTests 'scm-owner' @('-p','hiroute-platform','--test','windows_scm_owner') @(
    'native_standard_owner_credentials_select_profile_and_reader_failure_reverts_identity')
  $state = 'windows_background_fixture::state::tests::'
  Invoke-RequiredTests 'background-state' @('-p','hiroute-daemon','--test','windows_background',$state) @(
    ($state + 'post_uninstall_disposition_preserves_existing_state_and_unknown_additions'),
    ($state + 'post_uninstall_disposition_refuses_present_marker_without_touching_state'),
    ($state + 'former_recursive_state_cleanup_fails_the_preservation_oracle'),
    ($state + 'native_post_uninstall_preserves_file_ids_permissions_links_and_reparse_state'))
  $upgradeCleanup = 'windows_background_upgrade::transport::tests::standard_upgrade_cleanup_refuses_foreign_or_reparse_marker_and_preserves_state'
  Invoke-RequiredTests 'background-upgrade-cleanup' @('-p','hiroute-daemon','--test','windows_background',$upgradeCleanup) @($upgradeCleanup) @('--exact')
} elseif ($Scope -eq 'product') {
  cargo check --locked -p hiroute-daemon -p hiroute-cli --all-targets --keep-going 2>&1 | Tee-Object "$evidence/headless-build.log"
  if ($LASTEXITCODE -ne 0) { throw 'Full headless compilation failed; business scenarios are not executed' }
  Invoke-RequiredTests 'artifacts' @('-p','hiroute-local-storage','--lib','windows_artifacts_tests') @(
    'control::windows_artifacts_tests::windows_artifact_restart_restores_original_content_and_native_acl',
    'control::windows_artifacts_tests::windows_acl_change_loses_compensation_ownership_even_with_equal_content',
    'control::windows_artifacts_tests::windows_backup_authentication_binds_native_permission_facts',
    'control::windows_artifacts_tests::windows_owned_directory_cleanup_preserves_user_additions_and_replacements')
  New-Item -ItemType Directory -Force target/debug | Out-Null
  rustc --edition 2024 -C target-feature=+crt-static crates/cpa-bridge/tests/support/windows_cpa_fixture.rs -o target/debug/hiroute-cpa-process-fixture.exe
  if ($LASTEXITCODE -ne 0) { throw 'CPA fixture failed to build' }
  $env:HIROUTE_WINDOWS_CPA_FIXTURE = (Resolve-Path target/debug/hiroute-cpa-process-fixture.exe).Path
  Invoke-RequiredTests 'cpa' @('-p','hiroute-cpa-bridge','--lib','process::windows_tests') @(
    'process::windows_tests::native_bootstrap_delivers_stdin_eof_and_minimal_windows_environment',
    'process::windows_tests::native_root_exit_cannot_report_stopped_until_descendant_job_is_empty',
    'process::windows_tests::numeric_pid_is_never_an_orphan_termination_capability')
  Invoke-LibraryTests 'hiroute-cpa-bridge'
  cargo build --locked -p hiroute-cli --bin hiroute -p hiroute-daemon --bin hirouted 2>&1 | Tee-Object "$evidence/headless-link.log"
  if ($LASTEXITCODE -ne 0) { throw 'Production headless binaries failed to build' }
  $headless = 'installed_native_headless_roundtrip_and_graceful_then_forced_restart'
  $fragmented = 'server::core_runtime::adapters::tests::replay_ten_thousand_small_fields_use_one_stream_and_linear_template_scan'
  Invoke-RequiredTests 'replay-fragmentation' @('-p','hiroute-gateway','--lib',$fragmented) @($fragmented) @('--exact')
  Invoke-RequiredTests 'headless' @('-p','hiroute-daemon','--test','windows_headless',$headless) @($headless) @('--exact')
  $foregroundC = 'windows_foreground_console::actual_windows_service_run_ctrl_c_drains_the_gui_daemon_and_preserves_pipes'
  $foregroundBreak = 'windows_foreground_console::actual_windows_service_run_ctrl_break_drains_the_gui_daemon_and_preserves_pipes'
  Invoke-RequiredTests 'foreground-ctrl-c' @('-p','hiroute-daemon','--test','windows_headless',$foregroundC) @($foregroundC) @('--exact','--nocapture')
  Invoke-RequiredTests 'foreground-ctrl-break' @('-p','hiroute-daemon','--test','windows_headless',$foregroundBreak) @($foregroundBreak) @('--exact','--nocapture')
  Invoke-RequiredTests 'credential-options' @('-p','hiroute-cli','--lib','windows_handle_carrier_') @(
    'args::tests::windows_handle_carrier_rejects_missing_invalid_and_conflicting_authority',
    'args::tests::windows_handle_carrier_is_separate_from_a_typed_request_source')
  Invoke-RequiredTests 'worker-materials' @('-p','hiroute-daemon','--lib','windows_materials_') @(
    'delegation::local_worker::materials::tests::windows_materials_pin_root_and_cleanup_only_owned_children',
    'delegation::local_worker::materials::tests::windows_materials_refuse_an_existing_unowned_root')
  Invoke-RequiredTests 'worker-cleanup' @('-p','hiroute-daemon','--lib','windows_native_cleanup_') @(
    'delegation::native_cleanup::windows_tests::windows_native_cleanup_progress_preserves_marker_until_final_budget',
    'delegation::native_cleanup::windows_tests::windows_native_cleanup_refuses_replaced_marker_without_deleting_payload')
  Invoke-RequiredTests 'authority-key' @('-p','hiroute-daemon','--lib','authority_key_is_complete_stable') @(
    'control::runtime::observation_key::tests::authority_key_is_complete_stable_and_never_replaced_by_bad_lengths')
  Invoke-RequiredTests 'native-endpoint' @('-p','hiroute-daemon','--lib','windows_endpoint_fixture_') @(
    'control::runtime::agent_connection::discovery::endpoint_tests::windows_endpoint_fixture_registers_a_private_native_configuration')
  Invoke-RequiredTests 'native-home' @('-p','hiroute-daemon','--lib','windows_agent_discovery_uses_') @(
    'control::runtime::windows_agent_discovery_uses_the_private_isolated_test_home')
  Invoke-RequiredTests 'startup-admission' @('-p','hiroute-daemon','--bin','hirouted','standalone::bootstrap::tests::startup_reporting_failure_drains_actual_role_and_preserves_the_primary_error') @(
    'standalone::bootstrap::tests::startup_reporting_failure_drains_actual_role_and_preserves_the_primary_error') @('--exact')
  Invoke-RequiredTests 'live-capture' @('-p','hiroute-daemon','--lib','windows_live_capture_') @(
    'control::runtime::agent_live_check::windows::tests::windows_live_capture_observes_exit_and_bounds_final_stderr',
    'control::runtime::agent_live_check::windows::tests::windows_live_capture_timeout_stops_the_owned_tool_tree') @('--skip','windows_live_capture_fixture')
} elseif ($Scope -eq 'core') {
  # Entire native libraries, not an implementation-specific subset. Separate jobs
  # own platform primitives, CPA process fixtures and opt-in upstream clients.
  foreach ($package in (Get-WindowsCorePackages)) {
    Invoke-LibraryTests $package
  }
} elseif ($Scope -in @('background','upgrade','install')) {
  if (-not $InventoryOnly) {
  cargo build --locked -p hiroute-cli --bin hiroute -p hiroute-daemon --bin hirouted 2>&1 | Tee-Object "$evidence/headless-link.log"
  if ($LASTEXITCODE -ne 0) { throw 'Production background binaries failed to build' }
  if ($Scope -eq 'upgrade') {
    Initialize-WindowsAgentInputs
    $upgradeClients = Get-Content "$evidence/clients-result.json" -Raw | ConvertFrom-Json
    $env:HIROUTE_WINDOWS_UPGRADE_CODEX_BINARY = $upgradeClients.environment.HIROUTE_WORKER_CODEX_BINARY
    $env:HIROUTE_WINDOWS_UPGRADE_CODEX_SHA256 = $upgradeClients.binaries.HIROUTE_WORKER_CODEX_BINARY.sha256
    $env:HIROUTE_WINDOWS_UPGRADE_CODEX_VERSION = $upgradeClients.declarations.'@openai/codex'
  }
  $packageArgs = @('--expected-sha',$ExpectedSha,'--source-repo',$CpaSource,'--output',"$evidence/package",'--build-run-id',$env:GITHUB_RUN_ID)
  if ($CpaEvidence) {
    $packageArgs += @('--cpa-evidence',$CpaEvidence,'--cpa-original-run',$CpaOriginalRun,'--cpa-original-revision',$CpaOriginalRevision)
  }
  python scripts/windows-ci-package.py @packageArgs > "$evidence/package-result.json"
  if ($LASTEXITCODE -ne 0) { throw 'Production package prerequisites failed; background scenarios not executed' }
  $package = Get-Content "$evidence/package-result.json" -Raw | ConvertFrom-Json
  $env:HIROUTE_WINDOWS_PACKAGE_TARGET = $package.target
  $env:HIROUTE_WINDOWS_PACKAGE_MANIFEST = $package.manifest
  $env:HIROUTE_WINDOWS_PACKAGE_ARCHIVE = $package.archive
  $env:HIROUTE_WINDOWS_PACKAGE_DIRECTORY = $package.package_directory
  $env:HIROUTE_WINDOWS_ADMIN_CLI = $package.administrator_cli
  $env:HIROUTE_WORKER_DAEMON_BINARY = $package.worker_daemon
  }
  if ($Scope -eq 'install') {
    $case = 'task_native_background_lua_gateway_control_stop_crash_recovery_and_autostart'
    Invoke-RequiredTests 'native-install' @('-p','hiroute-daemon','--test','windows_background',$case) @($case) @('--exact','--nocapture')
  } elseif ($Scope -eq 'background') {
    Invoke-RequiredTests 'background-capture' @('-p','hiroute-daemon','--test','windows_background','windows_token_launcher::capture_tests::') @(
      'windows_token_launcher::capture_tests::native_capture_requires_last_writer_close_for_eof',
      'windows_token_launcher::capture_tests::native_capture_bounds_output_and_never_reports_unknown_stage_data')
    $packagedProxy = 'windows_packaged_foreground::proxy_observation::tests::'
    Invoke-RequiredTests 'packaged-proxy-oracle' @('-p','hiroute-daemon','--test','windows_headless',$packagedProxy) @(
      ($packagedProxy + 'only_exact_pinned_updater_origin_is_an_approved_rejected_connect'),
      ($packagedProxy + 'credentials_bodies_unknown_headers_and_ambiguous_hosts_stay_red'),
      ($packagedProxy + 'actual_loopback_connect_is_rejected_without_tls_then_observes_peer_eof'),
      ($packagedProxy + 'accepted_nonblocking_proxy_waits_for_delayed_peer_eof_after_rejection'),
      ($packagedProxy + 'empty_closed_socket_is_not_a_successful_proxy_observation'))
    $packaged = 'actual_windows_packaged_foreground_subscription_lifecycle'
    Invoke-RequiredTests 'packaged-foreground' @('-p','hiroute-daemon','--test','windows_headless',$packaged) @($packaged) @('--ignored','--exact','--nocapture')
    Invoke-RequiredTests 'background' @('-p','hiroute-daemon','--test','windows_background','task_native_background_lua_gateway_control_stop_crash_recovery_and_autostart') @(
      'task_native_background_lua_gateway_control_stop_crash_recovery_and_autostart') @('--exact','--nocapture')
    $standardOwner = 'windows_background_standard::task_real_standard_logged_on_owner_installs_runs_recovers_and_uninstalls'
    Invoke-RequiredTests 'background-standard-owner' @('-p','hiroute-daemon','--test','windows_background',$standardOwner) @($standardOwner) @('--exact','--nocapture')
    $standardUpgrade = 'windows_background_upgrade::task_real_standard_logged_on_owner_upgrade_recovers_rollback_and_preserves_business_state'
    Invoke-RequiredTests 'background-standard-upgrade' @('-p','hiroute-daemon','--test','windows_background',$standardUpgrade) @($standardUpgrade) @('--exact','--nocapture')
  } else {
    Initialize-WindowsUpgradeBaseline -ExpectedSha $ExpectedSha -BaselineRevision $BaselineRevision -BaselineManifest $BaselineManifest -BaselineArchive $BaselineArchive -Evidence $evidence
    Invoke-RequiredTests 'upgrade' @('-p','hiroute-daemon','--test','windows_background','windows_background_upgrade::task_native_candidate_upgrade_recovers_prepared_rollback_and_preserves_business_state') @(
      'windows_background_upgrade::task_native_candidate_upgrade_recovers_prepared_rollback_and_preserves_business_state') @('--exact','--nocapture')
  }
} elseif ($Scope -eq 'accounts') {
  # Account acceptance is explicitly selected after official login preparation.
  # Ordinary PR jobs never receive credentials or claim these cases executed.
  Initialize-WindowsAgentInputs
  foreach ($recipe in (Get-WindowsQoderAccountRecipes)) {
    Invoke-RequiredTests $recipe.name (@('-p',$recipe.package) + @($recipe.target) + @($recipe.case)) @($recipe.case) @('--ignored','--exact','--nocapture')
  }
} else {
  Invoke-RequiredTests 'agent-acp-notifications' @('-p','hiroute-daemon','--lib','delegation::acp::notification_tests::') @(
    'delegation::acp::notification_tests::startup_and_loaded_history_notifications_do_not_make_current_output_incomplete',
    'delegation::acp::notification_tests::wrong_session_notification_during_prompt_still_marks_output_incomplete')
  Invoke-RequiredTests 'agent-live-history' @('-p','hiroute-daemon','--lib','delegation::lifecycle::tests::terminal::') @(Get-WindowsLiveHistoryCases)
  $legacyHistory = 'delegation::profile::tests::materials_tests::legacy_private_history_retains_portable_paths_and_reopens_exact_session'
  Invoke-RequiredTests 'agent-legacy-history' @('-p','hiroute-daemon','--lib',$legacyHistory) @($legacyHistory) @('--exact')
  $userModelOracle = 'windows_user_models::native::diagnostic_tests::'
  Invoke-RequiredTests 'agent-user-model-oracle' @('-p','hiroute-daemon','--test','windows_headless',$userModelOracle) @(
    ($userModelOracle + 'public_check_phase_is_distinct_from_ordinary_native_model_invocation'),
    ($userModelOracle + 'public_failure_exposes_typed_status_and_code_without_private_native_fields'),
    ($userModelOracle + 'vendor_caption_classification_never_exposes_native_paths_or_guesses_numbers'),
    ($userModelOracle + 'sdk_after_failure_requires_the_exact_success_receipt_and_preserves_closed_stages'))
  $qoderUserOracle = 'windows_qoder_user_models::'
  Invoke-RequiredTests 'agent-qoder-user-model-oracle' @('-p','hiroute-daemon','--test','windows_headless',$qoderUserOracle) @(
    ($qoderUserOracle + 'native::tests::account_output_classification_never_reflects_native_error_text'),
    ($qoderUserOracle + 'native::tests::native_terminal_requires_exact_success_receipt_and_one_session'),
    ($qoderUserOracle + 'oracle::tests::ordinary_qoder_protocol_oracle_requires_independent_request_and_exact_usage'),
    ($qoderUserOracle + 'oracle::tests::ordinary_qoder_skill_oracle_rejects_uncorrelated_error_and_fake_cli_success'),
    ($qoderUserOracle + 'oracle::skill::tests::ordinary_qoder_skill_requires_one_exact_array_body_and_admitted_directory'),
    ($qoderUserOracle + 'oracle::skill::tests::ordinary_qoder_skill_native_binding_accepts_proven_drive_spelling_and_releases_restore'),
    ($qoderUserOracle + 'oracle::skill::tests::ordinary_qoder_skill_native_binding_rejects_foreign_reparse_links_and_changed_permissions'),
    ($qoderUserOracle + 'oracle::skill::tests::ordinary_qoder_bash_cli_preserves_stable_hardlink_identity_and_literal_spelling'),
    ($qoderUserOracle + 'settings::tests::ordinary_qoder_defaults_oracle_preserves_user_fields_and_jsonc_literals'))
  $qoderToolsShell = 'delegation::installation::installation_probe::native_tools_probe::shell_tests::'
  Invoke-RequiredTests 'agent-qoder-tools-shell' @('-p','hiroute-daemon','--lib',$qoderToolsShell) @(
    ($qoderToolsShell + 'qoder_tool_receipt_uses_bash_without_changing_other_native_shell_contracts'),
    ($qoderToolsShell + 'qoder_tool_bash_resources_bind_native_spellings_and_keep_empty_output_unproven'),
    ($qoderToolsShell + 'qoder_tools_repeat_with_borrowed_account_preserves_existing_skills_and_cleans_own'),
    ($qoderToolsShell + 'qoder_tool_final_effects_reject_late_same_identity_output_truncation'))
  $cancelOracle = 'delegation::installation::probe_server::tests::only_deliberately_cancelled_peer_disconnect_is_not_a_probe_response_failure'
  Invoke-RequiredTests 'agent-cancel-oracle' @('-p','hiroute-daemon','--lib',$cancelOracle) @($cancelOracle) @('--exact')
  $mainOracle = 'agents::collaboration_probe::tests::codex_collaboration_accepts_correlated_native_text_content_items_only'
  Invoke-RequiredTests 'agent-main-oracle' @('-p','hiroute-integrations','--lib',$mainOracle) @($mainOracle) @('--exact')
  $launchOracle = 'agents::collaboration_probe::tests::native_launch_diagnostic_'
  Invoke-RequiredTests 'agent-launch-oracle' @('-p','hiroute-integrations','--lib',$launchOracle) @(
    'agents::collaboration_probe::tests::native_launch_diagnostic_preserves_fixed_os_codes_and_api_stages',
    'agents::collaboration_probe::tests::native_launch_diagnostic_rejects_ambiguous_codes_and_never_emits_native_text')
  $launchCause = 'agents::collaboration_probe::launch_diagnostic::tests::'
  Invoke-RequiredTests 'agent-launch-cause' @('-p','hiroute-integrations','--lib',$launchCause) @(
    ($launchCause + 'arbitrary_digits_and_unfixed_api_names_are_not_classified'),
    ($launchCause + 'distinct_fixed_causes_and_codes_remain_ambiguous_and_private'),
    ($launchCause + 'fixed_windows_api_captions_preserve_only_their_own_codes'),
    ($launchCause + 'repeated_fixed_caption_keeps_one_cause_and_code'),
    ($launchCause + 'sandbox_preparation_context_does_not_invent_a_win32_error'),
    ($launchCause + 'wrapped_tool_error_is_not_assumed_to_be_an_os_create_failure'),
    ($launchCause + 'exec_policy_rejection_requires_native_wrapper_and_exact_fixed_reason'),
    ($launchCause + 'policy_caption_in_plain_or_quoted_command_does_not_claim_native_creation'),
    ($launchCause + 'nested_unified_create_retains_known_and_unknown_os_errors'),
    ($launchCause + 'policy_and_native_context_captions_remain_ambiguous'),
    ($launchCause + 'malformed_or_truncated_rejection_wrappers_stay_unknown'))
  $selectionInput = 'worker::selection_windows::tests::'
  $readonlyControl = 'agents::collaboration_probe::readonly_control::tests::'
  Invoke-RequiredTests 'agent-codex-readonly-control' @('-p','hiroute-integrations','--lib','readonly_') @(
    ($readonlyControl + 'readonly_control_requires_actual_attempt_and_access_denial_not_just_absence'),
    ($readonlyControl + 'readonly_control_rejects_echoed_policy_error_and_unrelated_receipts'),
    ($readonlyControl + 'readonly_control_rejects_reversed_or_repeated_receipt_lines'),
    ($readonlyControl + 'readonly_control_rejects_an_existing_file_and_any_write_before_or_after_receipt'),
    ($readonlyControl + 'readonly_control_preserves_a_literal_path_and_only_catches_access_denial'),
    ($readonlyControl + 'readonly_control_script_facts_are_closed_nonce_bound_and_never_satisfy_denial'),
    ($readonlyControl + 'readonly_control_script_facts_reject_duplicate_unknown_and_malformed_failure_rows'),
    ($readonlyControl + 'readonly_control_windows_parser_checks_actual_command_without_execution'),
    ($readonlyControl + 'readonly_control_console_diagnostics_preserve_exact_frames_and_cannot_replace_denial'),
    ($readonlyControl + 'readonly_pipeline_captures_before_console_failure_without_authorizing_denial'),
    ($readonlyControl + 'readonly_pipeline_rejects_echo_wrong_nonce_and_duplicate_frames'),
    ($readonlyControl + 'readonly_pipeline_rejects_unknown_and_noncanonical_failure_facts'),
    ($readonlyControl + 'readonly_pipeline_terminal_facts_survive_the_original_console_early_failure'),
    'agents::collaboration_probe::readonly_control::parser_probe::readonly_parser_trace_requires_ordered_actual_entry_parse_and_completion',
    'agents::collaboration_probe::readonly_control::parser_probe::readonly_parser_trace_limits_native_failure_facts_without_retaining_dynamic_output',
    'agents::collaboration_probe::readonly_control::parser_probe::readonly_parser_console_frames_preserve_parse_input_and_reject_echo_as_completion',
    'agents::collaboration_probe::tests::codex_readonly_control_precedes_the_skill_and_cli_and_requires_a_correlated_denial',
    'agents::collaboration_probe::tests::readonly_rejection_observes_closed_tool_facts_before_early_return',
    'agents::collaboration_probe::tests::readonly_facts_preserve_missing_correlation_and_echo_duplicate_gates',
    'agents::collaboration_probe::readonly_control::clm_tests::readonly_clm_requires_typed_filesystem_denial_and_known_matching_modes',
    'agents::collaboration_probe::readonly_control::clm_tests::readonly_clm_rejects_language_failure_duplicate_unknown_and_success_rows',
    'agents::collaboration_probe::readonly_control::clm_tests::readonly_script_preserves_full_language_and_uses_cmdlet_only_for_clm',
    'agents::collaboration_probe::readonly_control::windows_tests::readonly_windows_powershell_clm_writable_actual_write_is_rejected',
    'agents::collaboration_probe::readonly_control::windows_tests::readonly_windows_powershell_clm_acl_denial_is_accepted',
    'agents::collaboration_probe::readonly_control::windows_tests::readonly_windows_powershell_clm_method_error_cannot_authorize_denial',
    'agents::collaboration_probe::readonly_control::windows_tests::readonly_windows_pwsh_clm_writable_actual_write_is_rejected',
    'agents::collaboration_probe::readonly_control::windows_tests::readonly_windows_pwsh_clm_acl_denial_is_accepted',
    'agents::collaboration_probe::readonly_control::windows_tests::readonly_windows_pwsh_clm_method_error_cannot_authorize_denial')
  $toolDiagnostic = 'agents::collaboration_probe::tool_diagnostic::tests::'
  Invoke-RequiredTests 'agent-codex-tool-diagnostic' @('-p','hiroute-integrations','--lib',$toolDiagnostic) @(
    ($toolDiagnostic + 'official_terminal_and_live_headers_keep_numbers_but_discard_session_and_native_text'),
    ($toolDiagnostic + 'header_rejects_partial_duplicate_overflow_and_embedded_receipts'),
    ($toolDiagnostic + 'only_pinned_prelaunch_captions_establish_handler_preflight'),
    ($toolDiagnostic + 'facets_are_closed_and_shape_counts_do_not_invent_parsed_text'),
    ($toolDiagnostic + 'terminal_script_checkpoint_retains_failure_facts_without_proving_denial'))
  $qoderSkill = 'agents::qoder_probe::skill_diagnostic::tests::'
  Invoke-RequiredTests 'agent-qoder-skill-load-diagnostic' @('-p','hiroute-integrations','--lib',$qoderSkill) @(
    ($qoderSkill + 'skill_diagnostic_separates_shape_header_path_and_body_without_exposing_content'),
    ($qoderSkill + 'skill_diagnostic_separates_crlf_from_exact_body_and_rejects_missing_header_separator'),
    ($qoderSkill + 'skill_diagnostic_does_not_confuse_assistant_or_tool_output_with_loaded_user_text'),
    ($qoderSkill + 'windows_skill_diagnostic_binds_drive_spellings_to_one_actual_directory_and_content'),
    ($qoderSkill + 'windows_skill_diagnostic_rejects_foreign_reparse_device_and_ads_paths'),
    ($qoderSkill + 'windows_skill_diagnostic_detects_changed_content_on_the_same_retained_file'),
    ($qoderSkill + 'windows_skill_diagnostic_admits_only_original_or_proven_canonical_components'),
    ($qoderSkill + 'windows_loaded_skill_never_combines_foreign_body_shape_or_policy_facts'))
  $qoderHttp = 'agents::qoder_probe::http::tests::'
  Invoke-RequiredTests 'agent-qoder-skill-http-diagnostic' @('-p','hiroute-integrations','--lib',$qoderHttp) @(
    ($qoderHttp + 'installed_skill_requires_correlated_call_and_actual_user_source'),
    ($qoderHttp + 'model_claim_cannot_replace_cli_tool_evidence'),
    ($qoderHttp + 'native_cwd_envelope_keeps_the_correlated_exact_cli_proof'),
    ($qoderHttp + 'skill_receipt_diagnostics_do_not_admit_alternate_path_shape_or_newlines'),
    ($qoderHttp + 'windows_skill_receipt_requires_held_binding_and_same_candidate_exact_body'),
    ($qoderHttp + 'trusted_cli_still_rejects_semantic_only_framed_and_uncorrelated_outputs'))
  $qoderOutputRead = 'agents::qoder_probe::output_diagnostic_tests::'
  Invoke-RequiredTests 'agent-qoder-output-read-diagnostic' @('-p','hiroute-integrations','--lib',$qoderOutputRead) @(
    ($qoderOutputRead + 'qoder_final_read_diagnostic_distinguishes_native_oversize_from_path_open_failure'),
    ($qoderOutputRead + 'qoder_output_sampling_failure_keeps_retained_length_and_original_io'),
    ($qoderOutputRead + 'qoder_output_other_io_error_is_not_claimed_as_a_size_limit_and_success_is_unchanged'),
    ($qoderOutputRead + 'qoder_retained_private_output_rejects_reopen_without_losing_final_read_authority')) @('--nocapture')
  $retainedCapture = 'agents::native_probe_process::output_tests::'
  Invoke-RequiredTests 'agent-retained-capture' @('-p','hiroute-integrations','--lib',$retainedCapture) @(
    ($retainedCapture + 'retained_capture_starts_at_zero_and_rechecks_the_exact_final_bound'),
    ($retainedCapture + 'retained_capture_preserves_actual_non_limit_read_errors'))
  Invoke-RequiredTests 'agent-native-reader' @('-p','hiroute-local-storage','--lib','control::windows_native_reader_tests::') @(Get-WindowsNativeReaderCases)
  $profileIdentity = 'delegation::installation::windows_tests::'
  Invoke-RequiredTests 'agent-worker-profile-identity' @('-p','hiroute-daemon','--lib',$profileIdentity) @(
    ($profileIdentity + 'managed_new_profile_accepts_the_same_workspace_without_verbatim_spelling'),
    ($profileIdentity + 'native_root_new_to_continue_reopens_verbatim_base_with_exact_retained_identity'),
    ($profileIdentity + 'retained_native_root_rejects_a_junction_spelling_of_its_managed_base'))
  $directoryIdentity = 'delegation::profile::native_context::windows_tests::same_directory_accepts_verbatim_spelling_but_rejects_a_replaced_object'
  Invoke-RequiredTests 'agent-worker-directory-identity' @('-p','hiroute-daemon','--lib',$directoryIdentity) @($directoryIdentity) @('--exact')
  Invoke-RequiredTests 'agent-cli-selection' @('-p','hiroute-cli','--lib',$selectionInput) @(
    ($selectionInput + 'ordinary_inputs_bind_the_same_native_files_and_canonical_digest_without_mutation'),
    ($selectionInput + 'canonical_completed_selection_replays_before_missing_file_inspection'),
    ($selectionInput + 'selection_input_rejects_device_unc_streams_and_real_junctions_without_adoption'),
    ($selectionInput + 'ordinary_npm_shim_uses_the_shared_declared_bin_parser_and_rejects_reparse_bins'))
  $npmInspection = 'agents::windows_runtime::tests::windows_npm_inspection_runs_before_manifest_read_and_entry_canonicalization'
  Invoke-RequiredTests 'agent-npm-inspection' @('-p','hiroute-integrations','--lib',$npmInspection) @($npmInspection) @('--exact')
  $piLocator = 'agents::filesystem::pi_native::windows_tests::'
  Invoke-RequiredTests 'agent-pi-locator' @('-p','hiroute-integrations','--lib',$piLocator) @(
    ($piLocator + 'windows_pi_scanner_binds_local_npm_shim_to_declared_entry_without_evaluation'),
    ($piLocator + 'windows_pi_scanner_binds_global_npm_shim_to_declared_entry_without_evaluation'),
    ($piLocator + 'windows_pi_scanner_rejects_unknown_producer_escaping_bin_and_identity_mismatch'),
    ($piLocator + 'windows_pi_scanner_rejects_original_package_and_entry_junctions'),
    ($piLocator + 'windows_pi_direct_entry_ignores_outer_nonregular_manifest_after_nearest_package_match'))
  $historyOracle = 'delegation::installation::installation_probe::native_tools_probe::history_probe::tests::'
  Invoke-RequiredTests 'agent-history-oracle' @('-p','hiroute-daemon','--lib',$historyOracle) @(
    ($historyOracle + 'closed_history_shape_distinguishes_missing_failed_and_alternate_results'),
    ($historyOracle + 'closed_history_diagnostics_cannot_include_arbitrary_native_values'))
  $privateRoot = 'delegation::installation::installation_probe::windows_probe_root_creates_strict_private_directories_before_native_launch'
  Invoke-RequiredTests 'agent-private-root' @('-p','hiroute-daemon','--lib',$privateRoot) @($privateRoot) @('--exact','--nocapture')
  Initialize-WindowsAgentInputs
  $qoderBash = 'agents::qoder_probe::windows_cli::tests::windows_qoder_bash_executes_the_same_cli_with_literal_drive_path'
  Invoke-RequiredTests 'agent-qoder-cli-bash' @('-p','hiroute-integrations','--lib',$qoderBash) @($qoderBash) @('--ignored','--exact','--nocapture')
  $privateNode = 'agents::windows_runtime::tests::windows_selected_node_version_uses_the_private_capture_and_held_job'
  Invoke-RequiredTests 'agent-private-node' @('-p','hiroute-integrations','--lib',$privateNode) @($privateNode) @('--ignored','--exact','--nocapture')
  $pathIdentity = 'delegation::installation::installation_probe::windows_isolated_profile_uses_one_native_path_identity'
  Invoke-RequiredTests 'agent-path-identity' @('-p','hiroute-daemon','--lib',$pathIdentity) @($pathIdentity) @('--exact')
  $managedPath = 'agents::executable::windows_tests::windows_executable_observation_prepares_managed_launch_with_native_drive_identity'
  Invoke-RequiredTests 'agent-managed-path' @('-p','hiroute-integrations','--lib',$managedPath) @($managedPath) @('--exact')
  $accountFixture = 'delegation::installation::installation_probe::windows_acceptance::windows_qoder_account_fixture_rejects_missing_or_ambiguous_official_profile'
  Invoke-RequiredTests 'agent-account-fixture' @('-p','hiroute-daemon','--lib',$accountFixture) @($accountFixture) @('--exact')
  $piSdk = 'agents::windows_runtime::tests::windows_selected_pi_sdk_capabilities_load_the_actual_package'
  Invoke-RequiredTests 'agent-pi-sdk' @('-p','hiroute-integrations','--lib',$piSdk) @($piSdk) @('--ignored','--exact','--nocapture')
  $codexIngress = 'agents::native_ingress_probe::ephemeral::tests::real_codex_ephemeral_authentication_without_managed_configuration'
  Invoke-RequiredTests 'agent-native-codex-ingress' @('-p','hiroute-integrations','--lib',$codexIngress) @($codexIngress) @('--ignored','--exact','--nocapture')
  Invoke-RequiredTests 'agent-business-oracle' @('-p','hiroute-daemon','--test','windows_headless','windows_worker_business::oracle::tests::') @(
    'windows_worker_business::oracle::tests::a_prompt_or_unrelated_or_error_tool_result_cannot_supply_a_native_receipt',
    'windows_worker_business::oracle::tests::a_running_native_shell_does_not_complete_even_after_printing_json',
    'windows_worker_business::oracle::tests::ambiguous_native_cli_stdout_is_rejected')
  $httpOracle = 'windows_business_http::tests::windows_loopback_oracle_reads_delayed_header_and_body_from_nonblocking_listener'
  Invoke-RequiredTests 'agent-http' @('-p','hiroute-daemon','--test','windows_headless',$httpOracle) @($httpOracle) @('--exact')
  $nativeSourceOracle = 'windows_headless_business::upstream::tests::native_source_oracle_reads_delayed_header_and_body_from_nonblocking_stream'
  Invoke-RequiredTests 'agent-native-source-oracle' @('-p','hiroute-daemon','--test','windows_headless',$nativeSourceOracle) @($nativeSourceOracle) @('--exact')
  $dependencyOracle = 'windows_worker_business::dependency_selection::tests::'
  Invoke-RequiredTests 'agent-worker-dependency-oracle' @('-p','hiroute-daemon','--test','windows_headless',$dependencyOracle) @(
    ($dependencyOracle + 'closed_worker_diagnostic_never_returns_a_path_payload_or_unknown_key'),
    ($dependencyOracle + 'discovery_selection_requires_one_exact_held_native_file_identity'))
  $workerFailureOracle = 'windows_worker_business::failure_diagnostics::tests::'
  Invoke-RequiredTests 'agent-worker-failure-oracle' @('-p','hiroute-daemon','--test','windows_headless',$workerFailureOracle) @(
    ($workerFailureOracle + 'terminal_facts_publish_typed_states_and_correlations_without_result_or_locator_text'),
    ($workerFailureOracle + 'unknown_terminal_shapes_and_excessive_counts_remain_closed_and_unattributed'))
  $publicBusiness = 'actual_windows_complete_public_headless_business'
  $cancelPublication = 'windows_worker_business::cancel::tests::cancellation_waits_for_the_complete_native_receipt_after_file_creation'
  Invoke-RequiredTests 'agent-worker-cancel-publication' @('-p','hiroute-daemon','--test','windows_headless',$cancelPublication) @($cancelPublication) @('--exact')
  $residualBinding = 'windows_worker_business::cancel::tests::residual_binding_waits_for_complete_json_and_rejects_unreadable_native_identity'
  Invoke-RequiredTests 'agent-worker-residual-binding' @('-p','hiroute-daemon','--test','windows_headless',$residualBinding) @($residualBinding) @('--exact')
  $residualOracle = 'windows_worker_business::oracle::residual::tests::'
  Invoke-RequiredTests 'agent-worker-residual-oracle' @('-p','hiroute-daemon','--test','windows_headless',$residualOracle) @(
    ($residualOracle + 'residual_failure_caption_is_closed_and_keeps_unknown_vendor_output_unknown'),
    ($residualOracle + 'residual_tool_error_and_multiple_captions_cannot_disappear_as_no_result'),
    ($residualOracle + 'residual_script_keeps_literal_paths_and_original_sleep_before_late_write'))
  Invoke-RequiredTests 'agent-public-business' @('-p','hiroute-daemon','--test','windows_headless',$publicBusiness) @($publicBusiness) @('--ignored','--exact','--nocapture')
  $userModels = 'actual_windows_pi_and_dsh_persisted_user_models'
  Invoke-RequiredTests 'agent-user-models' @('-p','hiroute-daemon','--test','windows_headless',$userModels) @($userModels) @('--ignored','--exact','--nocapture')
  $management = 'actual_windows_installed_management_commands'
  $managementOracle = 'windows_management_business::tests::'
  Invoke-RequiredTests 'agent-management-oracle' @('-p','hiroute-daemon','--test','windows_headless',$managementOracle) @(
    ($managementOracle + 'management_daemon_unavailable_hint_requires_exact_negative_logs_envelope'),
    ($managementOracle + 'management_stderr_rejects_mixed_vendor_and_unknown_output'))
  Invoke-RequiredTests 'agent-management' @('-p','hiroute-daemon','--test','windows_headless',$management) @($management) @('--ignored','--exact','--nocapture')
  foreach ($case in (Get-WindowsDaemonAgentCases)) {
    $name = 'agent-' + ($case -split 'actual_')[1] -replace '_','-'
    Invoke-RequiredTests $name @('-p','hiroute-daemon','--lib',$case) @($case) @('--ignored','--exact','--nocapture')
  }
  foreach ($case in (Get-WindowsMainAgentCases)) {
    $name = 'main-' + ($case -split 'actual_windows_')[1] -replace '_','-'
    Invoke-RequiredTests $name @('-p','hiroute-integrations','--lib',$case) @($case) @('--ignored','--exact','--nocapture')
  }
}
} catch {
  $scopeExceptionFailed = $true
  throw
} finally {
  if ($InventoryOnly) {
    $requiredNames = @(Get-WindowsRequiredRecipes $Scope)
    $actualNames = @($inventoryRecipes | ForEach-Object { $_.name })
    if ($actualNames.Count -ne $requiredNames.Count -or
        (Compare-Object ($actualNames | Sort-Object) ($requiredNames | Sort-Object) -CaseSensitive)) { throw 'Complete source inventory required' }
    @{ schema='hiroute.windows-native-source-inventory/v1'; source_revision=$ExpectedSha;
       scope=$Scope; scenario='inventory_only'; runtime_executed=$false; recipes=@($inventoryRecipes.ToArray()) } |
      ConvertTo-Json -Depth 8 | Out-File -Encoding utf8 "$evidence/source-recipe-inventory.json"
  } else {
  if ($CaptureArtifacts) {
    $artifactArgs = @('finalize','--expected-sha',$ExpectedSha,'--evidence',$evidence)
    if ($Scope -in @('product','background','upgrade','agents','accounts')) { $artifactArgs += '--include-product-binaries' }
    if ($Scope -in @('background','upgrade','install') -and (Test-Path "$evidence/package-result.json") -and (Get-Item "$evidence/package-result.json").Length -gt 0) {
      $artifactArgs += @('--package-result',"$evidence/package-result.json")
    }
    python scripts/windows-native-artifacts.py @artifactArgs > "$evidence/artifact-finalization.json"
    if ($LASTEXITCODE -ne 0) { $failures.Add('Native replay artifacts could not be finalized') }
    else {
      python scripts/windows-native-artifacts.py verify --expected-sha $ExpectedSha --evidence $evidence > "$evidence/artifact-verification.json"
      if ($LASTEXITCODE -ne 0) { $failures.Add('Final native artifact verification failed') }
      else { $artifactVerified=$true }
      python scripts/windows-native-runtime-audit.py --expected-sha $ExpectedSha --evidence $evidence --require-static-crt > "$evidence/runtime-audit.json"
      if ($LASTEXITCODE -ne 0) { $failures.Add('Native imports require an unavailable Debug CRT or could not be verified') }
      else { $importsVerified=$true }
    }
  }
  @{ schema='hiroute.windows-native-scope-outcome/v1'; source_revision=$ExpectedSha; scope=$Scope;
     assessment=$Assessment; invocation_failed=($scopeExceptionFailed -or $failures.Count -gt 0);
     failure_count=$failures.Count; capture_requested=[bool]$CaptureArtifacts;
     artifact_verification_passed=$artifactVerified; runtime_import_audit_passed=$importsVerified;
     native_manifest_sha256=$(if($artifactVerified){(Get-FileHash -LiteralPath "$evidence/native-artifacts.json" -Algorithm SHA256).Hash.ToLowerInvariant()}else{$null})
  } | ConvertTo-Json -Depth 5 | Out-File -Encoding utf8 "$evidence/scope-outcome.json"
  python scripts/windows-ci-report.py $evidence
  if ($LASTEXITCODE -ne 0) { throw 'Native validation feedback could not be generated' }
  }
}
if ($failures.Count -gt 0) { throw ($failures -join '; ') }
