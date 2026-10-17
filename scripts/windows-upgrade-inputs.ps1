# Shared exact baseline trust boundary for native producer and Windows 11 replay.
function Initialize-WindowsUpgradeBaseline {
  param(
    [Parameter(Mandatory)][string]$ExpectedSha,
    [Parameter(Mandatory)][string]$BaselineRevision,
    [Parameter(Mandatory)][string]$BaselineManifest,
    [Parameter(Mandatory)][string]$BaselineArchive,
    [Parameter(Mandatory)][string]$Evidence
  )
  if ($ExpectedSha -notmatch '^[0-9a-f]{40}$' -or $BaselineRevision -notmatch '^[0-9a-f]{40}$' -or
      $ExpectedSha -ceq $BaselineRevision) { throw 'Two distinct exact committed upgrade packages required' }
  $manifest = (Resolve-Path -LiteralPath $BaselineManifest).Path
  $archive = (Resolve-Path -LiteralPath $BaselineArchive).Path
  python "$PSScriptRoot/windows-ci-package.py" --restage-only --expected-sha $ExpectedSha --package-revision $BaselineRevision --manifest $manifest --archive $archive --output "$Evidence/baseline-package" > "$Evidence/baseline-package-result.json"
  if ($LASTEXITCODE -ne 0) { throw 'Fresh baseline administrator trust could not be established' }
  $baseline = Get-Content -LiteralPath "$Evidence/baseline-package-result.json" -Raw | ConvertFrom-Json
  if ($baseline.source_revision -cne $BaselineRevision -or $baseline.consumer_revision -cne $ExpectedSha -or
      $baseline.target -cne $env:HIROUTE_WINDOWS_PACKAGE_TARGET -or
      $baseline.permission_source -cne 'fresh_verified_complete_extraction' -or
      $baseline.production_runtime -cne 'not_executed') { throw 'Baseline trust receipt differs from the explicit upgrade inputs' }
  $env:HIROUTE_WINDOWS_BASELINE_PACKAGE_MANIFEST = $manifest
  $env:HIROUTE_WINDOWS_BASELINE_PACKAGE_ARCHIVE = $archive
  $env:HIROUTE_WINDOWS_BASELINE_ADMIN_CLI = $baseline.administrator_cli
  $env:HIROUTE_WINDOWS_BASELINE_PACKAGE_DIRECTORY = $baseline.package_directory
}
