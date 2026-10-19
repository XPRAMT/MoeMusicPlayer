<#
Builds the smallest Android arm64 release APK (npm run android:build:arm64:release) and copies it to
release/update-assets/moemusicplayer-android-arm64.apk (ADR 0022). The app identifies an update only by the version in
the Release tag (v<version>, the same tag as the Windows package) and verifies the download against the SHA-256 digest
GitHub reports for the asset, so there is no manifest. Nothing is published unless -Upload -ReleaseTag <tag> is given
(uploads to that existing GitHub Release). The APK must be signed with the same key as the installed app, or Android
rejects the update.
#>
param(
    [string]$OutputDirectory,
    [string]$ReleaseTag,
    [switch]$Upload,
    [switch]$SkipBuild,
    [string]$Apk
)

$ErrorActionPreference = 'Stop'
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) { $OutputDirectory = Join-Path $repoRoot 'release\update-assets' }
if ([string]::IsNullOrWhiteSpace($Apk)) {
    $Apk = Join-Path $repoRoot 'src-tauri\gen\android\app\build\outputs\apk\universal\release\app-universal-release.apk'
}

Push-Location -LiteralPath $repoRoot
try {
    if (-not $SkipBuild) {
        $npmPath = (@(Get-Command 'npm.cmd' -CommandType Application -ErrorAction Stop))[0].Source
        & $npmPath run android:build:arm64:release
        if ($LASTEXITCODE -ne 0) { throw "Android build failed with exit code $LASTEXITCODE." }
    }
    if (-not (Test-Path -LiteralPath $Apk -PathType Leaf)) { throw "Missing APK: $Apk" }
    $version = ([System.IO.File]::ReadAllText((Join-Path $repoRoot 'src-tauri\build-version.txt'))).Trim()
    if ($version -notmatch '^\d{1,4}\.\d{1,4}\.\d{1,4}$') { throw "Invalid build version: $version" }

    $OutputDirectory = [System.IO.Path]::GetFullPath($OutputDirectory)
    New-Item -Path $OutputDirectory -ItemType Directory -Force | Out-Null
    $assetApk = Join-Path $OutputDirectory 'moemusicplayer-android-arm64.apk'
    Copy-Item -LiteralPath $Apk -Destination $assetApk -Force
    $staleManifest = Join-Path $OutputDirectory 'moemusicplayer-android-update.json'
    if (Test-Path -LiteralPath $staleManifest -PathType Leaf) { Remove-Item -LiteralPath $staleManifest -Force }
    Write-Output "APK: $assetApk"
    Write-Output "Version: $version  Release tag: v$version"
    if ($Upload) {
        if ([string]::IsNullOrWhiteSpace($ReleaseTag)) { throw '-ReleaseTag is required for explicit upload to an existing release.' }
        $gh = (@(Get-Command gh -CommandType Application -ErrorAction Stop))[0].Source
        & $gh release upload $ReleaseTag $assetApk --repo XPRAMT/MoeMusicPlayer --clobber
        if ($LASTEXITCODE -ne 0) { throw "Release upload failed with exit code $LASTEXITCODE." }
    }
}
finally {
    Pop-Location
}
