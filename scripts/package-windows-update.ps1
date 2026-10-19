<#
Packages the Windows release (ADR 0022): one zip, moemusicplayer-windows-x86_64.zip, holding moemusicplayer.exe. The
player identifies an update only by the version in the Release tag (v<version>, for example v26.10.19) and verifies the
download against the SHA-256 digest GitHub reports for the asset, so there is no manifest and no separate updater.
Nothing is published unless -Upload -ReleaseTag <tag> is given (uploads to that existing GitHub Release).
#>
param(
    [string]$Executable,
    [string]$OutputDirectory,
    [string]$ReleaseTag,
    [switch]$Upload
)

$ErrorActionPreference = 'Stop'
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($Executable)) { $Executable = Join-Path $repoRoot 'release\moemusicplayer.exe' }
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) { $OutputDirectory = Join-Path $repoRoot 'release\update-assets' }
if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) { throw "Missing release executable: $Executable" }
$Executable = (Resolve-Path -LiteralPath $Executable).ProviderPath
$OutputDirectory = [System.IO.Path]::GetFullPath($OutputDirectory)
if (-not (Test-Path -LiteralPath $OutputDirectory -PathType Container)) {
    New-Item -Path $OutputDirectory -ItemType Directory | Out-Null
}

# This mode exits before Tauri, settings, or SQLite initialize. Read the version from the exact executable being
# packaged, rather than the source checkout.
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
    $identity = $stdoutTask.Result | ConvertFrom-Json
}
finally { $process.Dispose() }
if ($identity.architecture -ne 'x86_64' -or $identity.buildVersion -notmatch '^\d{1,4}\.\d{1,4}\.\d{1,4}$' -or $identity.gitCommit -notmatch '^[0-9a-f]{40}$') {
    throw 'Only an identified Windows x64 build can be packaged.'
}

foreach ($stale in @('moemusicplayer.exe', 'moemusicplayer-updater.exe', 'moemusicplayer-update.json')) {
    $stalePath = Join-Path $OutputDirectory $stale
    if (Test-Path -LiteralPath $stalePath -PathType Leaf) { Remove-Item -LiteralPath $stalePath -Force }
}
$zipPath = Join-Path $OutputDirectory 'moemusicplayer-windows-x86_64.zip'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zipFile = [System.IO.File]::Open($zipPath, [System.IO.FileMode]::Create, [System.IO.FileAccess]::ReadWrite)
try {
    $archive = [System.IO.Compression.ZipArchive]::new($zipFile, [System.IO.Compression.ZipArchiveMode]::Create, $true)
    try {
        [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $Executable, 'moemusicplayer.exe', [System.IO.Compression.CompressionLevel]::Optimal) | Out-Null
    }
    finally { $archive.Dispose() }
}
finally { $zipFile.Dispose() }
$tag = "v$($identity.buildVersion)"
Write-Output "Package: $zipPath"
Write-Output "Version: $($identity.buildVersion)  Release tag: $tag  Build: $($identity.buildTimestampUtc)@$($identity.gitCommit)"
if ($Upload) {
    if ([string]::IsNullOrWhiteSpace($ReleaseTag)) { throw '-ReleaseTag is required for explicit upload to an existing release.' }
    $ghCommands = @(Get-Command gh -CommandType Application -ErrorAction Stop)
    & $ghCommands[0].Source release upload $ReleaseTag $zipPath --repo XPRAMT/MoeMusicPlayer --clobber
    if ($LASTEXITCODE -ne 0) { throw "Release upload failed with exit code $LASTEXITCODE." }
}
