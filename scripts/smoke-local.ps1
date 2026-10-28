$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
$taskBin = Join-Path $taskRoot '.tools/agave/4.3.2/solana-release/bin'
$taskValidatorExe = Join-Path $taskBin 'solana-test-validator.exe'
$taskKeygen = Join-Path $taskBin 'solana-keygen.exe'
$taskProgram = Join-Path $taskRoot 'env-smoke/target/deploy/environment_smoke.so'
$taskKeypair = Join-Path $taskRoot '.tools/env-smoke-keypair.json'
$taskRpc = 'http://127.0.0.1:18899'
$taskValidator = $null
foreach ($taskFile in @($taskValidatorExe, $taskKeygen, $taskProgram)) {
    if (-not (Test-Path -LiteralPath $taskFile)) { throw "Smoke prerequisite missing: $taskFile" }
}

# Refuse to use an existing listener; only test the validator started below.
$taskProbe = [System.Net.Sockets.TcpClient]::new()
try {
    $taskConnect = $taskProbe.ConnectAsync('127.0.0.1', 18899)
    try { $null = $taskConnect.Wait(500) } catch { }
    if ($taskProbe.Connected) { throw 'Port 18899 is already occupied. Stop that service or choose a separate test setup.' }
}
finally { $taskProbe.Dispose() }

if (-not (Test-Path -LiteralPath $taskKeypair)) {
    & $taskKeygen new --no-bip39-passphrase --silent --outfile $taskKeypair
    if ($LASTEXITCODE -ne 0) { throw 'Cannot create local smoke program identity.' }
}
$taskProgramId = (& $taskKeygen pubkey $taskKeypair).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Cannot read smoke program public key.' }
$taskRunDir = Join-Path $taskRoot ('.tools/smoke-runs/' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $taskRunDir -Force | Out-Null
$taskArgs = @('--ledger', ('"' + (Join-Path $taskRunDir 'ledger') + '"'),
    '--bind-address', '127.0.0.1', '--rpc-port', '18899', '--faucet-port', '18902',
    '--gossip-port', '18903', '--dynamic-port-range', '19000-19100', '--log',
    '--bpf-program', $taskProgramId, ('"' + $taskProgram + '"'))
try {
    $taskValidator = Start-Process -FilePath $taskValidatorExe -ArgumentList $taskArgs -WorkingDirectory $taskRoot `
        -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $taskRunDir 'stdout.log') `
        -RedirectStandardError (Join-Path $taskRunDir 'stderr.log')
    $taskReady = $false
    for ($taskTry = 0; $taskTry -lt 90; $taskTry++) {
        if ($taskValidator.HasExited) { throw "Validator exited. Inspect $taskRunDir" }
        try {
            $taskHealth = Invoke-RestMethod -Uri $taskRpc -Method Post -ContentType 'application/json' `
                -Body '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' -TimeoutSec 2
            if ($taskHealth.result -eq 'ok') { $taskReady = $true; break }
        } catch { }
        Start-Sleep -Milliseconds 500
    }
    if (-not $taskReady) { throw "Validator did not become healthy. Inspect $taskRunDir" }
    $taskReport = & node (Join-Path $PSScriptRoot 'smoke-client.mjs') $taskProgramId
    if ($LASTEXITCODE -ne 0) { throw "Smoke client failed. Inspect $taskRunDir" }
    $taskReport | Set-Content -LiteralPath (Join-Path $taskRunDir 'report.json') -Encoding utf8
    $taskReport
    Write-Output "Smoke evidence: $taskRunDir"
}
finally {
    if ($null -ne $taskValidator -and -not $taskValidator.HasExited) {
        Stop-Process -Id $taskValidator.Id -Force
    }
}
