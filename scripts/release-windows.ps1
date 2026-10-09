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
$previousBuildTimestamp = $env:MOE_BUILD_TIMESTAMP_UTC
$stagingExe = $null
$backupExe = $null
$stagingHelper = $null
$backupHelper = $null
$locationPushed = $false

Push-Location -LiteralPath $repoRoot
$locationPushed = $true
try {
    # Pin the build output directory so the source EXE is deterministic even when the caller
    # has an external or relative CARGO_TARGET_DIR. Restore the caller's process environment below.
    $env:CARGO_TARGET_DIR = $targetRoot
    $env:MOE_BUILD_TIMESTAMP_UTC = [DateTimeOffset]::UtcNow.ToString("yyyy-MM-dd'T'HH:mm:ss.fff'Z'")
    $helperBuildArguments = @('build', '--release', '-p', 'moemusicplayer-updater', '--bin', 'moemusicplayer-updater', '--locked')
    & cargo @helperBuildArguments
    if ($LASTEXITCODE -ne 0) { throw "Updater helper build failed with exit code $LASTEXITCODE." }

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
    $sourceHelper = Join-Path $releaseDirectory 'moemusicplayer-updater.exe'
    $outputHelper = Join-Path $outputDirectory 'moemusicplayer-updater.exe'
    $hadOutputMain = Test-Path -LiteralPath $outputExe -PathType Leaf
    $hadOutputHelper = Test-Path -LiteralPath $outputHelper -PathType Leaf
    if (-not (Test-Path -LiteralPath $sourceHelper -PathType Leaf)) { throw "Updater helper output missing: $sourceHelper" }
    $stagingHelper = Join-Path $outputDirectory ('.moemusicplayer-updater.' + [Guid]::NewGuid().ToString('N') + '.tmp')
    [System.IO.File]::Copy($sourceHelper, $stagingHelper, $false)
    if ((Get-Sha256Hex -Path $sourceHelper) -ne (Get-Sha256Hex -Path $stagingHelper)) { throw 'Updater helper staging failed SHA-256 verification. Existing release was left unchanged.' }

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
    try {
        if (Test-Path -LiteralPath $outputHelper -PathType Leaf) {
            $backupHelper = Join-Path $outputDirectory ('.moemusicplayer-updater.previous.' + [Guid]::NewGuid().ToString('N') + '.bak')
            [System.IO.File]::Replace($stagingHelper, $outputHelper, $backupHelper)
        }
        else { [System.IO.File]::Move($stagingHelper, $outputHelper) }
        $stagingHelper = $null
        if ((Get-Sha256Hex -Path $sourceHelper) -ne (Get-Sha256Hex -Path $outputHelper)) { throw 'Updater helper output failed SHA-256 verification.' }
    }
    catch {
        # A failure of the second file must not leave a partially published pair.
        if ($null -ne $backupExe -and (Test-Path -LiteralPath $backupExe -PathType Leaf)) {
            $mainRollback = Join-Path $outputDirectory ('.main-rollback.' + [Guid]::NewGuid().ToString('N') + '.tmp')
            [System.IO.File]::Copy($backupExe, $mainRollback, $false)
            [System.IO.File]::Replace($mainRollback, $outputExe, $null)
        }
        elseif (-not $hadOutputMain -and (Test-Path -LiteralPath $outputExe -PathType Leaf)) { Remove-Item -LiteralPath $outputExe -Force }
        if ($null -ne $backupHelper -and (Test-Path -LiteralPath $backupHelper -PathType Leaf)) {
            $helperRollback = Join-Path $outputDirectory ('.updater-rollback.' + [Guid]::NewGuid().ToString('N') + '.tmp')
            [System.IO.File]::Copy($backupHelper, $helperRollback, $false)
            if (Test-Path -LiteralPath $outputHelper -PathType Leaf) { [System.IO.File]::Replace($helperRollback, $outputHelper, $null) }
            else { [System.IO.File]::Move($helperRollback, $outputHelper) }
        }
        elseif (-not $hadOutputHelper -and (Test-Path -LiteralPath $outputHelper -PathType Leaf)) { Remove-Item -LiteralPath $outputHelper -Force }
        throw
    }
    if ($null -ne $backupExe -and (Test-Path -LiteralPath $backupExe -PathType Leaf)) {
        try {
            Remove-Item -LiteralPath $backupExe -Force -ErrorAction Stop
            $backupExe = $null
        }
        catch {
            $cleanupException = $_.Exception
            $fileAccessException = $null
            while ($null -ne $cleanupException) {
                if ($cleanupException -is [System.IO.IOException] -or $cleanupException -is [System.UnauthorizedAccessException]) {
                    $fileAccessException = $cleanupException
                    break
                }
                $cleanupException = $cleanupException.InnerException
            }
            if ($null -eq $fileAccessException) {
                throw
            }
            Write-Warning "Previous executable backup is still in use or cannot be removed; the verified release was updated. Keeping backup: '$backupExe'."
        }
    }
    Write-Output "Updated: $outputExe"
    Write-Output "SHA256:  $outputHash"
    if ($null -ne $backupHelper -and (Test-Path -LiteralPath $backupHelper -PathType Leaf)) { Remove-Item -LiteralPath $backupHelper -Force }
    & (Join-Path $scriptDirectory 'package-windows-update.ps1') -Executable $outputExe -Helper $outputHelper
}
finally {
    if ($null -ne $previousBuildTimestamp) {
        $env:MOE_BUILD_TIMESTAMP_UTC = $previousBuildTimestamp
    }
    else {
        Remove-Item -LiteralPath 'Env:MOE_BUILD_TIMESTAMP_UTC' -ErrorAction SilentlyContinue
    }
    if ($null -ne $stagingExe -and (Test-Path -LiteralPath $stagingExe -PathType Leaf)) {
        Remove-Item -LiteralPath $stagingExe -Force -ErrorAction SilentlyContinue
    }
    if ($null -ne $stagingHelper -and (Test-Path -LiteralPath $stagingHelper -PathType Leaf)) { Remove-Item -LiteralPath $stagingHelper -Force -ErrorAction SilentlyContinue }
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
