$ErrorActionPreference = 'Stop'

function Get-Sha256Hex([string]$Path) {
    $stream = [System.IO.File]::OpenRead($Path)
    try {
        $sha256 = [System.Security.Cryptography.SHA256]::Create()
        try {
            $hashBytes = $sha256.ComputeHash($stream)
            return [System.BitConverter]::ToString($hashBytes).Replace('-', '')
        }
        finally {
            $sha256.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}

$scriptDirectory = (Resolve-Path -LiteralPath $PSScriptRoot).ProviderPath
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $scriptDirectory '..'))
$targetRoot = [System.IO.Path]::GetFullPath((Join-Path $repoRoot 'target'))
$hadCargoTargetDir = $null -ne (Get-Item -LiteralPath 'Env:CARGO_TARGET_DIR' -ErrorAction SilentlyContinue)
$previousCargoTargetDir = if ($hadCargoTargetDir) { $env:CARGO_TARGET_DIR } else { $null }
$stagingExe = $null
$backupExe = $null
$locationPushed = $false

Push-Location -LiteralPath $repoRoot
$locationPushed = $true
try {
    # Pin the build output directory so the source EXE is deterministic even when the caller
    # has an external or relative CARGO_TARGET_DIR. Restore the caller's process environment below.
    $env:CARGO_TARGET_DIR = $targetRoot

    $npmCommands = @(Get-Command 'npm.cmd' -CommandType Application -ErrorAction Stop)
    if ($npmCommands.Count -eq 0) {
        throw "npm.cmd was not found on PATH. Existing release executable was left unchanged."
    }
    $npmPath = $npmCommands[0].Source
    $buildArguments = @('run', 'tauri', '--', 'build', '--no-bundle', '--ci')
    & $npmPath @buildArguments
    $buildExitCode = $LASTEXITCODE
    if ($buildExitCode -ne 0) {
        throw "Tauri Release build failed with exit code $buildExitCode. Existing release executable was left unchanged."
    }

    $releaseDirectory = Join-Path $targetRoot 'release'
    if (-not [string]::IsNullOrWhiteSpace($env:CARGO_BUILD_TARGET)) {
        $releaseDirectory = Join-Path (Join-Path $targetRoot $env:CARGO_BUILD_TARGET) 'release'
    }
    $sourceExe = Join-Path $releaseDirectory 'moemusicplayer.exe'
    if (-not (Test-Path -LiteralPath $sourceExe -PathType Leaf)) {
        throw "Tauri build succeeded but source executable was not found at '$sourceExe'. Existing release executable was left unchanged."
    }

    $sourceInfo = Get-Item -LiteralPath $sourceExe
    if ($sourceInfo.Length -le 0) {
        throw "Source executable is empty: '$sourceExe'. Existing release executable was left unchanged."
    }

    $outputDirectory = Join-Path $repoRoot 'release'
    if (-not (Test-Path -LiteralPath $outputDirectory -PathType Container)) {
        New-Item -Path $outputDirectory -ItemType Directory | Out-Null
    }
    $outputExe = Join-Path $outputDirectory 'moemusicplayer.exe'
    $stagingExe = Join-Path $outputDirectory ('.moemusicplayer.' + [Guid]::NewGuid().ToString('N') + '.tmp')

    [System.IO.File]::Copy($sourceExe, $stagingExe, $false)
    $stagingInfo = Get-Item -LiteralPath $stagingExe
    if ($stagingInfo.Length -ne $sourceInfo.Length) {
        throw "Staged executable size does not match the successful build output. Existing release executable was left unchanged."
    }
    $sourceHash = Get-Sha256Hex -Path $sourceExe
    $stagingHash = Get-Sha256Hex -Path $stagingExe
    if ($stagingHash -ne $sourceHash) {
        throw "Staged executable hash does not match the successful build output. Existing release executable was left unchanged."
    }

    if (Test-Path -LiteralPath $outputExe -PathType Leaf) {
        $backupExe = Join-Path $outputDirectory ('.moemusicplayer.previous.' + [Guid]::NewGuid().ToString('N') + '.bak')
        [System.IO.File]::Replace($stagingExe, $outputExe, $backupExe)
    }
    else {
        [System.IO.File]::Move($stagingExe, $outputExe)
    }
    $stagingExe = $null

    $outputInfo = Get-Item -LiteralPath $outputExe
    if ($outputInfo.Length -ne $sourceInfo.Length) {
        throw "Updated executable size does not match the build output: '$outputExe'."
    }
    $outputHash = Get-Sha256Hex -Path $outputExe
    if ($outputHash -ne $sourceHash) {
        throw "Updated executable hash does not match the build output: '$outputExe'."
    }
    if ($null -ne $backupExe -and (Test-Path -LiteralPath $backupExe -PathType Leaf)) {
        Remove-Item -LiteralPath $backupExe -Force
        $backupExe = $null
    }
    Write-Output "Updated: $outputExe"
    Write-Output "SHA256:  $outputHash"
}
finally {
    if ($null -ne $stagingExe -and (Test-Path -LiteralPath $stagingExe -PathType Leaf)) {
        Remove-Item -LiteralPath $stagingExe -Force -ErrorAction SilentlyContinue
    }
    if ($hadCargoTargetDir) {
        $env:CARGO_TARGET_DIR = $previousCargoTargetDir
    }
    else {
        Remove-Item -LiteralPath 'Env:CARGO_TARGET_DIR' -ErrorAction SilentlyContinue
    }
    if ($locationPushed) {
        Pop-Location
    }
}
