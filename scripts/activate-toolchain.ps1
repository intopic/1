# Dot-source from PowerShell: . ./scripts/activate-toolchain.ps1
# Changes only this PowerShell process and its future child processes.
param([switch]$IncludeAnchor)

$projectToolchainRoot = Split-Path -Parent $PSScriptRoot
$projectAnchorBin = Join-Path $projectToolchainRoot '.tools/anchor/1.2.1'
$projectAgaveBin = Join-Path $projectToolchainRoot '.tools/agave/4.3.2/solana-release/bin'

$projectActivationPaths = @($projectAgaveBin)
if ($IncludeAnchor) {
    $projectManifest = Get-Content -LiteralPath (Join-Path $projectToolchainRoot 'toolchain.versions.json') -Raw | ConvertFrom-Json
    if ($projectManifest.anchor_execution_status -eq 'BLOCKED_BY_WINDOWS_ANTIVIRUS') {
        throw 'Native Anchor execution is blocked by Windows antivirus. Use the documented WSL setup; no bypass is configured.'
    }
    $projectActivationPaths += $projectAnchorBin
}
foreach ($projectRequiredExecutable in @((Join-Path $projectAgaveBin 'solana.exe'))) {
    if (-not (Test-Path -LiteralPath $projectRequiredExecutable -PathType Leaf)) {
        throw "Pinned toolchain file missing: $projectRequiredExecutable. Finish the reviewed local installation before activating."
    }
}

# Keep activation idempotent without changing registry/user/machine PATH.
$projectExistingPath = @($env:PATH -split ';' | Where-Object {
    $_ -and $_ -ne $projectAnchorBin -and $_ -ne $projectAgaveBin
})
$env:PATH = ($projectActivationPaths + $projectExistingPath) -join ';'
Write-Host 'Activated project-local Agave 4.3.2 for this process. Native Anchor remains blocked; see docs/toolchain.md.'
