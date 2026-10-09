param(
    [string]$Executable,
    [string]$Helper,
    [string]$OutputDirectory,
    [string]$ReleaseTag,
    [switch]$Upload
)

$ErrorActionPreference = 'Stop'
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($Executable)) { $Executable = Join-Path $repoRoot 'release\moemusicplayer.exe' }
if ([string]::IsNullOrWhiteSpace($Helper)) { $Helper = Join-Path $repoRoot 'release\moemusicplayer-updater.exe' }
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) { $OutputDirectory = Join-Path $repoRoot 'release\update-assets' }
foreach ($inputPath in @($Executable, $Helper)) {
    if (-not (Test-Path -LiteralPath $inputPath -PathType Leaf)) { throw "Missing release executable: $inputPath" }
}
$Executable = (Resolve-Path -LiteralPath $Executable).ProviderPath
$Helper = (Resolve-Path -LiteralPath $Helper).ProviderPath
$OutputDirectory = [System.IO.Path]::GetFullPath($OutputDirectory)
if (-not (Test-Path -LiteralPath $OutputDirectory -PathType Container)) {
    New-Item -Path $OutputDirectory -ItemType Directory | Out-Null
}

# This mode exits before Tauri, settings, or SQLite initialize. Read identity
# from the exact executable being packaged, rather than the source checkout.
$startInfo = [System.Diagnostics.ProcessStartInfo]::new()
$startInfo.FileName = $Executable
$startInfo.Arguments = '--update-build-info'
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $true
$startInfo.RedirectStandardOutput = $true
$startInfo.RedirectStandardError = $true
$process = [System.Diagnostics.Process]::Start($startInfo)
$stdoutTask = $process.StandardOutput.ReadToEndAsync()
$stderrTask = $process.StandardError.ReadToEndAsync()
try {
    if (-not $process.WaitForExit(10000)) { throw 'Build identity command did not finish within 10 seconds.' }
    if ($process.ExitCode -ne 0) { throw "Build identity command failed: $($stderrTask.Result)" }
    $identityText = $stdoutTask.Result
    $identity = $identityText | ConvertFrom-Json
    # PowerShell 7 can materialize ISO strings as DateTime; preserve the exact
    # UTC millisecond string embedded in the executable (also works in PS 5.1).
    $timestampMatch = [regex]::Match($identityText, '"buildTimestampUtc"\s*:\s*"(?<stamp>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z)"')
    if (-not $timestampMatch.Success) { throw 'Executable did not provide a precise UTC build timestamp.' }
    $identity.buildTimestampUtc = $timestampMatch.Groups['stamp'].Value
}
finally { $process.Dispose() }
if ($identity.architecture -ne 'x86_64' -or $identity.buildTimestampUtc -notmatch '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$' -or $identity.gitCommit -notmatch '^[0-9a-f]{40}$') {
    throw 'Only an identified Windows x64 build can be packaged.'
}
$manifest = [ordered]@{
    schemaVersion = 1
    version = $identity.buildVersion
    buildTimestampUtc = $identity.buildTimestampUtc
    gitCommit = $identity.gitCommit
    architecture = 'windows-x86_64'
    executable = [ordered]@{
        name = 'moemusicplayer.exe'
        sha256 = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash.ToLowerInvariant()
        size = (Get-Item -LiteralPath $Executable).Length
    }
    helper = [ordered]@{
        name = 'moemusicplayer-updater.exe'
        sha256 = (Get-FileHash -LiteralPath $Helper -Algorithm SHA256).Hash.ToLowerInvariant()
        size = (Get-Item -LiteralPath $Helper).Length
    }
}
$manifestPath = Join-Path $OutputDirectory 'moemusicplayer-update.json'
# serde_json expects a JSON document without a BOM.
[System.IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 8), [System.Text.UTF8Encoding]::new($false))
$assetExe = Join-Path $OutputDirectory 'moemusicplayer.exe'
$assetHelper = Join-Path $OutputDirectory 'moemusicplayer-updater.exe'
if ($Executable -ne $assetExe) { Copy-Item -LiteralPath $Executable -Destination $assetExe -Force }
if ($Helper -ne $assetHelper) { Copy-Item -LiteralPath $Helper -Destination $assetHelper -Force }
$zipPath = Join-Path $OutputDirectory 'moemusicplayer-windows-x86_64.zip'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zipFile = [System.IO.File]::Open($zipPath, [System.IO.FileMode]::Create, [System.IO.FileAccess]::ReadWrite)
try {
    $archive = [System.IO.Compression.ZipArchive]::new($zipFile, [System.IO.Compression.ZipArchiveMode]::Create, $true)
    try {
        foreach ($assetPath in @($assetExe, $assetHelper, $manifestPath)) {
            [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $assetPath, [System.IO.Path]::GetFileName($assetPath)) | Out-Null
        }
    }
    finally { $archive.Dispose() }
}
finally { $zipFile.Dispose() }
Write-Output "Package: $zipPath"
Write-Output "Build: $($identity.buildTimestampUtc)@$($identity.gitCommit)"
Write-Output "Manifest SHA256: $((Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant())"
if ($Upload) {
    if ([string]::IsNullOrWhiteSpace($ReleaseTag)) { throw '-ReleaseTag is required for explicit upload to an existing release.' }
    $ghCommands = @(Get-Command gh -CommandType Application -ErrorAction Stop)
    $uploadArguments = @('release', 'upload', $ReleaseTag, $assetExe, $assetHelper, $manifestPath, $zipPath, '--repo', 'XPRAMT/MoeMusicPlayer', '--clobber')
    & $ghCommands[0].Source @uploadArguments
    if ($LASTEXITCODE -ne 0) { throw "Release upload failed with exit code $LASTEXITCODE." }
}
