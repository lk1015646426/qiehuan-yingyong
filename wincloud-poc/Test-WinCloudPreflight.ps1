#requires -Version 5.1
<#
.SYNOPSIS
  Read-only readiness inspection for a Windows cloud desktop WorkBuddy test.
.DESCRIPTION
  Only checks operating system, capacity, interactive session, process presence,
  loopback service availability, and optional Tailscale executable presence.
  It never reads credential stores, Windows user profiles, WorkBuddy auth files,
  SSH keys, registry secrets, browser cookies, or API keys. It does not install
  software, change system settings, start apps, or contact external websites.
  -FixturePath feeds synthetic evidence and does NOT touch the local computer.
  This tool cannot certify WorkBuddy licensing, API permissions, cloud budget,
  Windows Server compatibility, unattended session behavior, or DSH Go.
#>
[CmdletBinding()]
param(
    [string]$FixturePath,
    [switch]$Json
)
$ErrorActionPreference = 'Stop'

function Test-LocalPool {
    try {
        $null = Invoke-WebRequest -Uri 'http://127.0.0.1:18789/v1/models' -TimeoutSec 2 -UseBasicParsing -ErrorAction Stop
        return $true
    } catch {
        $response = $_.Exception.Response
        if ($null -ne $response -and [int]$response.StatusCode -eq 401) {
            # A challenge response is evidence of a listener only, NOT of a working model.
            return $true
        }
        return $false
    }
}

function Get-ReadOnlyEvidence {
    $os = Get-CimInstance -ClassName Win32_OperatingSystem -ErrorAction Stop
    $computer = Get-CimInstance -ClassName Win32_ComputerSystem -ErrorAction Stop
    $systemDrive = $env:SystemDrive
    if (-not $systemDrive) { $systemDrive = 'C:' }
    $disk = Get-CimInstance -ClassName Win32_LogicalDisk -Filter "DeviceID='$systemDrive'"
    $tail = Get-Command -Name 'tailscale.exe' -ErrorAction SilentlyContinue
    if (-not $tail) {
        $tailPath = Join-Path $env:ProgramFiles 'Tailscale\tailscale.exe'
        $tail = Test-Path -LiteralPath $tailPath -PathType Leaf
    }
    # A running process is not proof that a real WorkBuddy login is usable.
    $client = @(Get-Process -Name 'WorkBuddy','CodeBuddy' -ErrorAction SilentlyContinue).Count -gt 0
    return [pscustomobject]@{
        OsCaption = [string]$os.Caption
        MemoryGB = [math]::Round($os.TotalVisibleMemorySize / 1MB, 2)
        CpuCores = [int]$computer.NumberOfLogicalProcessors
        DiskFreeGB = if ($disk) { [math]::Round($disk.FreeSpace / 1GB, 2) } else { 0 }
        InteractiveSession = [bool]([Environment]::UserInteractive -and $env:SESSIONNAME -ne 'Services')
        ClientProcessDetected = [bool]$client
        PoolLoopbackResponds = [bool](Test-LocalPool)
        TailscaleInstalled = [bool]$tail
    }
}

function New-Check([string]$Id, [string]$State, [string]$Message) {
    return [pscustomobject]@{ Id=$Id; State=$State; Note=$Message }
}
function Test-WinCloudEvidence([object]$Evidence, [bool]$Synthetic) {
    $checks = @()
    $caption = [string]$Evidence.OsCaption
    if ($caption -match 'Windows 10|Windows 11') {
        $checks += New-Check 'OFFICIAL_OS' 'PASS' 'WorkBuddy officially lists Windows 10+ desktop x64.'
    } elseif ($caption -match 'Windows Server') {
        $checks += New-Check 'OFFICIAL_OS' 'REVIEW' 'Windows Server edition not guaranteed by published WorkBuddy desktop requirements; manual test required.'
    } else {
        $checks += New-Check 'OFFICIAL_OS' 'FAIL' 'Cannot establish supported Windows desktop OS.'
    }
    if ([double]$Evidence.MemoryGB -ge 4) {
        $checks += New-Check 'RAM_4G' 'PASS' 'At least 4 GB RAM; this is a testing heuristic, not an official minimum.'
    } else {
        $checks += New-Check 'RAM_4G' 'FAIL' 'Less than 4 GB RAM; unsuitable first-choice VM for GUI Windows + WorkBuddy.'
    }
    if ([int]$Evidence.CpuCores -ge 2) {
        $checks += New-Check 'CPU_2' 'PASS' 'At least 2 logical CPUs.'
    } else {
        $checks += New-Check 'CPU_2' 'FAIL' 'Less than 2 logical CPUs.'
    }
    if ([double]$Evidence.DiskFreeGB -ge 15) {
        $checks += New-Check 'DISK_15G' 'PASS' 'At least 15 GiB free on system drive (testing heuristic).'
    } else {
        $checks += New-Check 'DISK_15G' 'FAIL' 'Insufficient free system-drive capacity for a cautious desktop test.'
    }
    if ($Evidence.InteractiveSession -eq $true) {
        $checks += New-Check 'INTERACTIVE' 'PASS' 'Interactive Windows user session is present.'
    } else {
        $checks += New-Check 'INTERACTIVE' 'FAIL' 'No interactive Windows user session verified; GUI auto-start is not assured.'
    }
    if ($Evidence.ClientProcessDetected -eq $true) {
        $checks += New-Check 'WORKBUDDY_PROCESS' 'PASS' 'A candidate WorkBuddy process is running; login status remains unverified.'
    } else {
        $checks += New-Check 'WORKBUDDY_PROCESS' 'REVIEW' 'WorkBuddy process not detected; install/login/GUI launch not yet established.'
    }
    if ($Evidence.PoolLoopbackResponds -eq $true) {
        $checks += New-Check 'POOL_LOOPBACK' 'PASS' 'Something replied on 127.0.0.1:18789; no model or token was used.'
    } else {
        $checks += New-Check 'POOL_LOOPBACK' 'REVIEW' 'No HTTP listener evidence from 127.0.0.1:18789.'
    }
    if ($Evidence.TailscaleInstalled -eq $true) {
        $checks += New-Check 'TAILSCALE' 'PASS' 'Tailscale executable installed (not proof of login or connectivity).'
    } else {
        $checks += New-Check 'TAILSCALE' 'REVIEW' 'Tailscale not installed; install and authenticate manually if using private HTTPS.'
    }

    $verdict = 'READY_FOR_INTERACTIVE_SMOKE_TEST_ONLY'
    if (@($checks | Where-Object { $_.State -eq 'FAIL' }).Count -gt 0) {
        $verdict = 'NOT_READY'
    } elseif (@($checks | Where-Object { $_.State -eq 'REVIEW' }).Count -gt 0) {
        $verdict = 'NEEDS_MANUAL_VERIFICATION'
    }
    return [pscustomobject]@{
        Schema = 'workbuddy.wincloud.preflight.v1'
        FixtureOnly = $Synthetic
        Environment = 'Windows desktop/VM'
        Verdict = $verdict
        ProductionReady = $false
        Observations = @($checks)
        Unverified = @(
            'Windows Server support', 'WorkBuddy interactive login and client lifecycle',
            'pool model response with authorized local credentials',
            'cloud provider billing and free credits',
            'provider terms on personal session use', 'HTTPS tailnet accessibility',
            'Android DSH Go actual integration', 'behavior after RDP disconnect and VM reboot'
        )
        Disclaimer = 'No real WorkBuddy model requests or credentials were used; this is not a production readiness certificate.'
    }
}

$synthetic = -not [string]::IsNullOrWhiteSpace($FixturePath)
if ($synthetic) {
    $absolute = (Resolve-Path -LiteralPath $FixturePath).Path
    $evidence = Get-Content -LiteralPath $absolute -Raw -Encoding UTF8 | ConvertFrom-Json
} else {
    if ($env:OS -ne 'Windows_NT') { throw 'Live inspection requires Windows; use -FixturePath on other OS.' }
    $evidence = Get-ReadOnlyEvidence
}
$result = Test-WinCloudEvidence -Evidence $evidence -Synthetic $synthetic
if ($Json) {
    $result | ConvertTo-Json -Depth 6
} else {
    Write-Output ("WorkBuddy Windows cloud test readiness: " + $result.Verdict)
    foreach ($check in $result.Observations) {
        Write-Output ('[{0}] {1} - {2}' -f $check.State, $check.Id, $check.Note)
    }
    Write-Output 'ProductionReady=False. Interactive manual tests, permitted usage, cost and Android connection are unverified.'
    if ($synthetic) { Write-Output 'WARNING: SYNTHETIC FIXTURE, not a real VM.' }
}
