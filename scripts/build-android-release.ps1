<#
Builds the Android arm64 release as a small APK (about 26 MB) and an AAB for Google Play (ADR 0023): fat LTO, one
codegen unit, stripped, optimized for size. The profile settings are passed as cargo environment overrides, so the
Windows release profile in Cargo.toml is untouched.

Signing: both files are signed with the release key. The key comes from the environment, nothing secret is in the
repository:
  MOE_ANDROID_KEYSTORE            default C:\APP\@Android\Play KEY.jks
  MOE_ANDROID_KEY_ALIAS           default "anime theme key"
  MOE_ANDROID_KEYSTORE_PASSWORD   default: the "Password" line of C:\APP\@Android\Play KEY.txt
  MOE_ANDROID_KEY_PASSWORD        default: the keystore password
MOE_ANDROID_DEBUG_KEY=1 signs with the debug key instead (the key of the apps installed before ADR 0023, so the result
updates them in place); its files get a "-debugkey" suffix and never overwrite the release-key ones.
Without a keystore the build falls back to the debug key (a build anyone can make; it cannot update an app that was
installed with the release key, and Google Play rejects it).

The version name is the build version (src-tauri\build-version.txt, for example 26.10.19) and the version code is its
date as a number (26.10.19 becomes 261019), so every day's upload to Google Play has a higher version code. Output:
  release\moemusicplayer-android-arm64-release.apk
  release\moemusicplayer-android-arm64-release.aab   (uploaded to Google Play by hand)
-Fast: the same signed APK without the slow size optimizations (no LTO, 16 codegen units; still stripped and
opt-level s), only the APK (no AAB), written as release\moemusicplayer-android-arm64-fast.apk. For everyday small
changes; it installs over the full build because the signing key is the same.
#>
param([switch]$Fast)
$ErrorActionPreference = 'Stop'
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))

function Get-NotesPassword([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $null }
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    # keytool's output saved by a Traditional Chinese Windows console (code page 950), or UTF-8.
    $text = try { [System.Text.UTF8Encoding]::new($false, $true).GetString($bytes) } catch { [System.Text.Encoding]::GetEncoding(950).GetString($bytes) }
    foreach ($line in ($text -split "`r?`n")) {
        if ($line -match '^\s*(?i:password|密碼)\s*[:：=]\s*(?<value>\S.*?)\s*$') { return $Matches['value'] }
    }
    return $null
}

Push-Location -LiteralPath $repoRoot
$overrides = [ordered]@{
    CARGO_PROFILE_RELEASE_LTO = 'true'
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS = '1'
    CARGO_PROFILE_RELEASE_STRIP = 'true'
    CARGO_PROFILE_RELEASE_OPT_LEVEL = 's'
}
if ($Fast) { $overrides['CARGO_PROFILE_RELEASE_LTO'] = 'false'; $overrides['CARGO_PROFILE_RELEASE_CODEGEN_UNITS'] = '16' }
$configPath = $null
$saved = @{}
try {
    # ---- version name and code from the build version
    $version = ([System.IO.File]::ReadAllText((Join-Path $repoRoot 'src-tauri\build-version.txt'))).Trim()
    if ($version -notmatch '^(?<y>\d{1,4})\.(?<m>\d{1,2})\.(?<d>\d{1,2})$') { throw "Invalid build version: $version" }
    $versionCode = [int]$Matches['y'] * 10000 + [int]$Matches['m'] * 100 + [int]$Matches['d']
    $configPath = Join-Path ([System.IO.Path]::GetTempPath()) ("moe-android-version-" + [Guid]::NewGuid().ToString('N') + '.json')
    [System.IO.File]::WriteAllText($configPath, (@{ version = $version; bundle = @{ android = @{ versionCode = $versionCode } } } | ConvertTo-Json -Depth 5), [System.Text.UTF8Encoding]::new($false))

    # ---- signing key from the environment (or the maintainer's defaults)
    $suffix = ''
    if ($env:MOE_ANDROID_DEBUG_KEY -eq '1') { $suffix = '-debugkey' }
    if ($suffix -ne '') { Write-Output 'Signing with the debug key (MOE_ANDROID_DEBUG_KEY=1).' }
    elseif ([string]::IsNullOrWhiteSpace($env:MOE_ANDROID_KEYSTORE)) {
        $defaultKeystore = 'C:\APP\@Android\Play KEY.jks'
        if (Test-Path -LiteralPath $defaultKeystore -PathType Leaf) { $overrides['MOE_ANDROID_KEYSTORE'] = $defaultKeystore }
    }
    else { $overrides['MOE_ANDROID_KEYSTORE'] = $env:MOE_ANDROID_KEYSTORE }
    if ($suffix -ne '') { $overrides['MOE_ANDROID_KEYSTORE'] = '' }  # no release key: Gradle falls back to the debug key
    elseif ($overrides.Contains('MOE_ANDROID_KEYSTORE')) {
        $overrides['MOE_ANDROID_KEY_ALIAS'] = if ($env:MOE_ANDROID_KEY_ALIAS) { $env:MOE_ANDROID_KEY_ALIAS } else { 'anime theme key' }
        $password = if ($env:MOE_ANDROID_KEYSTORE_PASSWORD) { $env:MOE_ANDROID_KEYSTORE_PASSWORD } else { Get-NotesPassword 'C:\APP\@Android\Play KEY.txt' }
        if ([string]::IsNullOrEmpty($password)) { throw 'No keystore password: set MOE_ANDROID_KEYSTORE_PASSWORD.' }
        $overrides['MOE_ANDROID_KEYSTORE_PASSWORD'] = $password
        if ($env:MOE_ANDROID_KEY_PASSWORD) { $overrides['MOE_ANDROID_KEY_PASSWORD'] = $env:MOE_ANDROID_KEY_PASSWORD }
        Write-Output "Signing with the release key (alias $($overrides['MOE_ANDROID_KEY_ALIAS']))."
    }
    else { Write-Warning 'No release keystore found: signing with the debug key (not accepted by Google Play).' }

    foreach ($name in $overrides.Keys) { $saved[$name] = [Environment]::GetEnvironmentVariable($name); [Environment]::SetEnvironmentVariable($name, [string]$overrides[$name]) }
    $npx = (@(Get-Command 'npx.cmd' -CommandType Application -ErrorAction Stop))[0].Source
    if ($Fast) { & $npx tauri android build --target aarch64 --apk --ci --config $configPath }
    else { & $npx tauri android build --target aarch64 --apk --aab --ci --config $configPath }
    if ($LASTEXITCODE -ne 0) { throw "Android release build failed with exit code $LASTEXITCODE." }

    $outputs = Join-Path $repoRoot 'src-tauri\gen\android\app\build\outputs'
    $apk = Join-Path $outputs 'apk\universal\release\app-universal-release.apk'
    if (-not (Test-Path -LiteralPath $apk -PathType Leaf)) { throw "Missing APK: $apk" }
    if ($Fast) {
        Copy-Item -LiteralPath $apk -Destination (Join-Path $repoRoot "release\moemusicplayer-android-arm64-fast$suffix.apk") -Force
        Write-Output "Version $version, version code $versionCode"
        Write-Output "APK: release\moemusicplayer-android-arm64-fast$suffix.apk"
        return
    }
    Copy-Item -LiteralPath $apk -Destination (Join-Path $repoRoot "release\moemusicplayer-android-arm64-release$suffix.apk") -Force
    $aab = Get-ChildItem -LiteralPath (Join-Path $outputs 'bundle') -Recurse -Filter '*.aab' -File -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($null -eq $aab) { throw 'Missing AAB under src-tauri\gen\android\app\build\outputs\bundle.' }
    Copy-Item -LiteralPath $aab.FullName -Destination (Join-Path $repoRoot "release\moemusicplayer-android-arm64-release$suffix.aab") -Force
    Write-Output "Version $version, version code $versionCode"
    Write-Output "APK: release\moemusicplayer-android-arm64-release$suffix.apk"
    Write-Output "AAB: release\moemusicplayer-android-arm64-release$suffix.aab"
}
finally {
    foreach ($name in $overrides.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }
    if ($null -ne $configPath -and (Test-Path -LiteralPath $configPath)) { Remove-Item -LiteralPath $configPath -Force }
    Pop-Location
}
