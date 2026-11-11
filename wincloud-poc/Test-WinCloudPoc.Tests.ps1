#requires -Version 5.1
<#
  Self-contained tests for WorkBuddy Windows cloud readiness.
  Every fixture is synthetic. No real account, network, install or VM touched.
#>
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$script = Join-Path $root 'Test-WinCloudPreflight.ps1'

function Assert-Equal([object]$Actual,[object]$Expected,[string]$Label) {
    if ($Actual -ne $Expected) {
        throw "FAIL $Label expected '$Expected', actual '$Actual'"
    }
    Write-Output "PASS $Label"
}
function Test-Fixture([string]$Name,[string]$ExpectedVerdict,[string]$ExpectedOS,[string]$ExpectedRam) {
    $fixture = Join-Path (Join-Path $root 'fixtures') $Name
    $raw = & $script -FixturePath $fixture -Json
    $test = $raw | ConvertFrom-Json
    Assert-Equal $test.Schema 'workbuddy.wincloud.preflight.v1' "$Name schema"
    Assert-Equal $test.FixtureOnly $true "$Name synthetic only"
    Assert-Equal $test.Verdict $ExpectedVerdict "$Name verdict"
    Assert-Equal $test.ProductionReady $false "$Name not production-approved"
    Assert-Equal (@($test.Observations).Count) 8 "$Name checks count"
    $os = @($test.Observations | Where-Object { $_.Id -eq 'OFFICIAL_OS' })[0]
    Assert-Equal $os.State $ExpectedOS "$Name OS risk"
    $ram = @($test.Observations | Where-Object { $_.Id -eq 'RAM_4G' })[0]
    Assert-Equal $ram.State $ExpectedRam "$Name RAM risk"
    if ($raw -match 'Bearer |access_token|refresh_token|workbuddy-desktop.info|secure-account-storage.key') {
        throw "FAIL $Name confidential material in output"
    }
}

Test-Fixture 'windows11-ready.json' 'READY_FOR_INTERACTIVE_SMOKE_TEST_ONLY' 'PASS' 'PASS'
Test-Fixture 'windows-server-review.json' 'NEEDS_MANUAL_VERIFICATION' 'REVIEW' 'PASS'
Test-Fixture 'azure-b1s-lowmem.json' 'NOT_READY' 'REVIEW' 'FAIL'
$scriptText = Get-Content -LiteralPath (Join-Path $root 'Test-WinCloudPreflight.ps1') -Raw -Encoding UTF8
if ($scriptText -match 'ReadAllBytes|Copy-Item|Set-ItemProperty|Get-Content.*workbuddy-desktop.info') {
    throw 'FAIL: unsafe privileged or credential file operation in preflight'
}
$tailnet = Join-Path $root 'Enable-PrivateTailnet.ps1'
$preview = & $tailnet
if (($preview -join [Environment]::NewLine) -notmatch 'No changes made') {
    throw 'FAIL: Serve dry-run did not say no changes'
}
if (($preview -join [Environment]::NewLine) -match 'tailscale funnel') {
    throw 'FAIL: Serve dry-run mentions public Funnel'
}
Write-Output 'PASS Private Serve command is preview-only without -Apply'
Write-Output 'ALL TESTS PASSED'
