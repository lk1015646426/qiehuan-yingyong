#requires -Version 5.1
<#
.SYNOPSIS
  Opt-in private Tailscale Serve mapping for an already running local WorkBuddy pool.
.DESCRIPTION
  By default prints instructions and makes no changes. Requires -Apply to change
  Tailscale Serve configuration; SupportsShouldProcess means -WhatIf and -Confirm.
  Never opens Windows public firewall ports or configures Tailscale Funnel.
  Does not install Tailscale, log into accounts or access WorkBuddy credentials.
  WorkBuddy pool itself must already be started through its normal desktop UI.
#>
[CmdletBinding(SupportsShouldProcess=$true, ConfirmImpact='High')]
param(
    [switch]$Apply,
    [ValidateRange(1,65535)]
    [int]$LocalPort = 18789
)
$ErrorActionPreference = 'Stop'
if ($LocalPort -ne 18789) { throw 'Only the reviewed 127.0.0.1:18789 WorkBuddy pool port is supported.' }

$cmd = "tailscale serve --bg --https=443 http://127.0.0.1:$LocalPort"
if (-not $Apply) {
    Write-Output 'Preview: private Tailscale Serve route; accessible only to authorized tailnet devices.'
    Write-Output $cmd
    Write-Output 'No changes made. After a real VM test, run with -Apply and approve the prompt.'
    return
}

if ($env:OS -ne 'Windows_NT') { throw 'Windows cloud VM required.' }
$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Open an elevated PowerShell console on the Windows VM to configure Tailscale Serve.'
}
$exe = Get-Command 'tailscale.exe' -ErrorAction SilentlyContinue
if ($exe) {
    $tail = $exe.Source
} else {
    $tail = Join-Path $env:ProgramFiles 'Tailscale\tailscale.exe'
    if (-not (Test-Path -LiteralPath $tail -PathType Leaf)) {
        throw 'Tailscale not installed. Install from tailscale.com/download and sign in yourself.'
    }
}

# Only an unauthenticated local GET: no key, no model request, no account access.
$responds = $false
try {
    $null = Invoke-WebRequest -Uri "http://127.0.0.1:$LocalPort/v1/models" -UseBasicParsing -TimeoutSec 3 -ErrorAction Stop
    $responds = $true
} catch {
    if ($null -ne $_.Exception.Response -and [int]$_.Exception.Response.StatusCode -eq 401) {
        $responds = $true
    }
}
if (-not $responds) {
    throw 'No WorkBuddy pool loopback service detected. Start the normal desktop pool first.'
}
if ($PSCmdlet.ShouldProcess('private tailnet HTTPS port 443', 'Map to WorkBuddy pool localhost only')) {
    & $tail serve --bg --https=443 "http://127.0.0.1:$LocalPort"
    if ($LASTEXITCODE -ne 0) {
        throw 'Tailscale Serve did not confirm a successful configuration.'
    }
    & $tail serve status
    Write-Output 'Configured private tailnet HTTPS. This does NOT prove the model works when RDP disconnects.'
    Write-Output 'Do not use tailscale funnel. Keep the WorkBuddy API key private.'
}
