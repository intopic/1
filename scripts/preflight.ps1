param([switch]$RequireSolana)

$ErrorActionPreference = 'Stop'
$taskRequired = @('rustc', 'cargo')
if ($RequireSolana) { $taskRequired += @('solana', 'anchor') }
$taskInventory = @('rustc', 'cargo', 'node', 'python', 'git', 'solana', 'anchor', 'wsl')
$taskMissing = @()
$taskProjectRoot = Split-Path -Parent $PSScriptRoot
$taskLocalTools = @{
    solana = (Join-Path $taskProjectRoot '.tools/agave/4.3.2/solana-release/bin/solana.exe')
    anchor = (Join-Path $taskProjectRoot '.tools/anchor/1.2.1/anchor.exe')
}

foreach ($taskTool in $taskInventory) {
    $taskCommand = Get-Command $taskTool -ErrorAction SilentlyContinue
    $taskFound = $null -ne $taskCommand
    $taskPath = if ($taskFound) { $taskCommand.Source } else { '(missing)' }
    $taskStatus = if ($taskFound) { 'command found; execution not checked' } else { 'missing' }
    if (-not $taskFound -and $taskLocalTools.ContainsKey($taskTool) -and
        (Test-Path -LiteralPath $taskLocalTools[$taskTool])) {
        $taskFound = $true
        $taskPath = $taskLocalTools[$taskTool]
        $taskStatus = 'project-local file; activate first'
        if ($taskTool -eq 'anchor') {
            $taskFound = $false
            $taskStatus = 'BLOCKED by Windows antivirus; use WSL'
        }
    }
    [PSCustomObject]@{
        Tool = $taskTool
        Available = $taskFound
        RequiredNow = $taskTool -in $taskRequired
        Path = $taskPath
        Status = $taskStatus
    }
    if (-not $taskFound -and $taskTool -in $taskRequired) { $taskMissing += $taskTool }
}

Write-Output ''
Write-Output 'This check does not install tools or verify WSL distro, SBF build, RPC, wallet, or transaction-v1 support.'
if ($taskMissing.Count -gt 0) {
    Write-Error ('Missing required commands: ' + ($taskMissing -join ', '))
    exit 1
}
Write-Output 'Required commands are present. Run cargo test --workspace --offline next.'
