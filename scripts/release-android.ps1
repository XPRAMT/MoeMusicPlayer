<#
Builds the Android arm64 APK with a pinned build identity and packages the in-app update assets
(ADR 0015) into release/update-assets:
  moemusicplayer-android-arm64.apk
  moemusicplayer-android-update.json   (schemaVersion 1, architecture android-arm64)
Nothing is published unless -Upload -ReleaseTag <tag> is given (uploads to that existing GitHub
Release, next to the Windows assets). The APK must be signed with the same key as the installed
app, or Android rejects the update.
#>
param(
    [string]$OutputDirectory,
    [string]$ReleaseTag,
    [switch]$Upload,
    [switch]$SkipBuild,
    [string]$Apk
)

$ErrorActionPreference = 'Stop'
function Get-PackageSha256([string]$Path) {
    $stream = [System.IO.File]::OpenRead($Path)
    try {
        $sha256 = [System.Security.Cryptography.SHA256]::Create()
        try { return [System.BitConverter]::ToString($sha256.ComputeHash($stream)).Replace('-', '').ToLowerInvariant() }
        finally { $sha256.Dispose() }
    }
    finally { $stream.Dispose() }
}

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) { $OutputDirectory = Join-Path $repoRoot 'release\update-assets' }
if ([string]::IsNullOrWhiteSpace($Apk)) {
    $Apk = Join-Path $repoRoot 'src-tauri\gen\android\app\build\outputs\apk\universal\debug\app-universal-debug.apk'
}
$identityPath = Join-Path $repoRoot 'release\android-build-identity.json'

Push-Location -LiteralPath $repoRoot
$previousBuildTimestamp = $env:MOE_BUILD_TIMESTAMP_UTC
try {
    if (-not $SkipBuild) {
        $gitCommit = (& git rev-parse HEAD).Trim()
        if ($LASTEXITCODE -ne 0 -or $gitCommit -notmatch '^[0-9a-f]{40}$') { throw 'Could not read the git commit.' }
        # build.rs embeds this exact UTC string; the app compares it with the manifest.
        $env:MOE_BUILD_TIMESTAMP_UTC = [DateTimeOffset]::UtcNow.ToString("yyyy-MM-dd'T'HH:mm:ss.fff'Z'")
        $npmPath = (@(Get-Command 'npm.cmd' -CommandType Application -ErrorAction Stop))[0].Source
        & $npmPath run android:build:arm64
        if ($LASTEXITCODE -ne 0) { throw "Android build failed with exit code $LASTEXITCODE." }
        $identity = [ordered]@{ buildTimestampUtc = $env:MOE_BUILD_TIMESTAMP_UTC; gitCommit = $gitCommit }
        New-Item -Path (Split-Path $identityPath) -ItemType Directory -Force | Out-Null
        [System.IO.File]::WriteAllText($identityPath, ($identity | ConvertTo-Json), [System.Text.UTF8Encoding]::new($false))
    }
    if (-not (Test-Path -LiteralPath $Apk -PathType Leaf)) { throw "Missing APK: $Apk" }
    if (-not (Test-Path -LiteralPath $identityPath -PathType Leaf)) { throw 'Missing build identity; run without -SkipBuild first.' }
    $identityText = [System.IO.File]::ReadAllText($identityPath)
    $stampMatch = [regex]::Match($identityText, '"buildTimestampUtc"\s*:\s*"(?<stamp>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z)"')
    $commitMatch = [regex]::Match($identityText, '"gitCommit"\s*:\s*"(?<commit>[0-9a-f]{40})"')
    if (-not $stampMatch.Success -or -not $commitMatch.Success) { throw 'Invalid Android build identity.' }
    $version = ([System.IO.File]::ReadAllText((Join-Path $repoRoot 'src-tauri\build-version.txt'))).Trim()
    if ([string]::IsNullOrWhiteSpace($version)) { throw 'Missing build version.' }

    $OutputDirectory = [System.IO.Path]::GetFullPath($OutputDirectory)
    New-Item -Path $OutputDirectory -ItemType Directory -Force | Out-Null
    $assetApk = Join-Path $OutputDirectory 'moemusicplayer-android-arm64.apk'
    Copy-Item -LiteralPath $Apk -Destination $assetApk -Force
    $manifest = [ordered]@{
        schemaVersion = 1
        version = $version
        buildTimestampUtc = $stampMatch.Groups['stamp'].Value
        gitCommit = $commitMatch.Groups['commit'].Value
        architecture = 'android-arm64'
        apk = [ordered]@{
            name = 'moemusicplayer-android-arm64.apk'
            sha256 = Get-PackageSha256 -Path $assetApk
            size = (Get-Item -LiteralPath $assetApk).Length
        }
    }
    $manifestPath = Join-Path $OutputDirectory 'moemusicplayer-android-update.json'
    # serde_json expects a JSON document without a BOM.
    [System.IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 8), [System.Text.UTF8Encoding]::new($false))
    Write-Output "APK: $assetApk"
    Write-Output "Build: $($manifest.buildTimestampUtc)@$($manifest.gitCommit)"
    Write-Output "Manifest SHA256: $(Get-PackageSha256 -Path $manifestPath)"
    if ($Upload) {
        if ([string]::IsNullOrWhiteSpace($ReleaseTag)) { throw '-ReleaseTag is required for explicit upload to an existing release.' }
        $gh = (@(Get-Command gh -CommandType Application -ErrorAction Stop))[0].Source
        & $gh release upload $ReleaseTag $assetApk $manifestPath --repo XPRAMT/MoeMusicPlayer --clobber
        if ($LASTEXITCODE -ne 0) { throw "Release upload failed with exit code $LASTEXITCODE." }
    }
}
finally {
    $env:MOE_BUILD_TIMESTAMP_UTC = $previousBuildTimestamp
    Pop-Location
}
