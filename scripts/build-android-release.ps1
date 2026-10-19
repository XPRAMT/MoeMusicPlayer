<#
Builds the smallest Android arm64 release APK (about 26 MB; the debug build is over 100 MB):
fat LTO, one codegen unit, stripped, optimized for size. The profile settings are passed as cargo environment
overrides, so the Windows release profile in Cargo.toml is untouched. The APK is signed with the debug key (the
installed app's key), so it updates the installed app in place (ADR 0015). Output:
  src-tauri\gen\android\app\build\outputs\apk\universal\release\app-universal-release.apk
copied to release\moemusicplayer-android-arm64-release.apk.
#>
$ErrorActionPreference = 'Stop'
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Push-Location -LiteralPath $repoRoot
$saved = @{}
$overrides = @{
    CARGO_PROFILE_RELEASE_LTO = 'true'
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS = '1'
    CARGO_PROFILE_RELEASE_STRIP = 'true'
    CARGO_PROFILE_RELEASE_OPT_LEVEL = 's'
}
try {
    foreach ($name in $overrides.Keys) { $saved[$name] = [Environment]::GetEnvironmentVariable($name); [Environment]::SetEnvironmentVariable($name, $overrides[$name]) }
    $npx = (@(Get-Command 'npx.cmd' -CommandType Application -ErrorAction Stop))[0].Source
    & $npx tauri android build --target aarch64 --apk --ci
    if ($LASTEXITCODE -ne 0) { throw "Android release build failed with exit code $LASTEXITCODE." }
    $apk = Join-Path $repoRoot 'src-tauri\gen\android\app\build\outputs\apk\universal\release\app-universal-release.apk'
    if (-not (Test-Path -LiteralPath $apk -PathType Leaf)) { throw "Missing APK: $apk" }
    Copy-Item -LiteralPath $apk -Destination (Join-Path $repoRoot 'release\moemusicplayer-android-arm64-release.apk') -Force
}
finally {
    foreach ($name in $overrides.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }
    Pop-Location
}
