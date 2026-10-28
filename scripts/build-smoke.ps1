$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
$taskAgaveBin = Join-Path $taskRoot '.tools/agave/4.3.2/solana-release/bin'
$taskDriver = Join-Path $taskAgaveBin 'cargo-build-sbf.exe'
if (-not (Test-Path -LiteralPath $taskDriver)) { throw 'Pinned Agave build driver is missing.' }
Push-Location -LiteralPath $taskRoot
try {
    $taskOldPath = $env:PATH
    $env:PATH = $taskAgaveBin + ';' + $env:PATH
    & $taskDriver --manifest-path env-smoke/Cargo.toml --tools-version v1.58 --skip-tools-install --offline
    if ($LASTEXITCODE -ne 0) { throw 'SBF smoke build failed.' }
}
finally {
    $env:PATH = $taskOldPath
    Pop-Location
}
