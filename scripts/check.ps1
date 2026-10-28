$ErrorActionPreference = 'Stop'
$taskProjectRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $taskProjectRoot
try {
    & cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw 'Rust formatting check failed.' }
    & cargo test --workspace --offline
    if ($LASTEXITCODE -ne 0) { throw 'Rust model tests failed.' }
    & cargo clippy --workspace --all-targets --offline -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Rust clippy check failed.' }
    Write-Output 'Local model checks passed. No SVM or network benchmark was run.'
}
finally {
    Pop-Location
}
