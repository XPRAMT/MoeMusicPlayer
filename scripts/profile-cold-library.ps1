<#
.SYNOPSIS
  Profile a cold library build (no SQLite database at all -> every track tagged and stored), ADR 0027.
.DESCRIPTION
  Runs the headless example `profile_cold_library` N times, each into a brand-new output directory, so neither the
  running player nor its UserData database is touched; the music folder is only read. While each run is going, the
  process CPU time, working set and I/O byte counters are sampled with UTC timestamps (samples.csv) so they line up with
  the in-process phase events (profile.jsonl). The run ends at the `library_ready` event / library_ready.json marker.
  Run 1 is reported as "first" (whatever the OS file cache holds), later runs as "warm"; caches are never flushed.
.EXAMPLE
  .\scripts\profile-cold-library.ps1 -MusicRoot D:\Music -Runs 3
#>
param(
    [string]$MusicRoot = 'D:\Music',
    [int]$Runs = 3,
    [string]$OutRoot = (Join-Path $env:TEMP ('moe-cold-profile\' + (Get-Date -Format 'yyyyMMdd-HHmmss'))),
    [int]$SampleMs = 250,
    [switch]$NoBuild,
    [switch]$KeepDatabases
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $repo 'target\release\examples\profile_cold_library.exe'
if (-not $NoBuild) {
    Push-Location $repo
    try {
        cargo build --release -p player-platform-windows --example profile_cold_library
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
    } finally { Pop-Location }
}
if (-not (Test-Path $exe)) { throw "missing $exe" }
New-Item -ItemType Directory -Force -Path $OutRoot | Out-Null
$env:MOE_LIBRARY_PROFILE_UTC_OFFSET_MINUTES = [string][int][TimeZoneInfo]::Local.GetUtcOffset([DateTime]::Now).TotalMinutes
Remove-Item Env:MOE_LIBRARY_PROFILE -ErrorAction SilentlyContinue

function Read-Events([string]$path) {
    Get-Content -LiteralPath $path -Encoding UTF8 | Where-Object { $_.Trim() } | ForEach-Object { $_ | ConvertFrom-Json }
}

$summaries = @()
for ($run = 1; $run -le $Runs; $run++) {
    $dir = Join-Path $OutRoot ("run-{0:D2}" -f $run)
    $samples = Join-Path $dir 'samples.csv'
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    "utc_ms,cpu_ms,working_set_bytes,read_bytes,write_bytes,read_ops,threads" | Set-Content -LiteralPath $samples -Encoding UTF8
    $proc = Start-Process -FilePath $exe -ArgumentList @("`"$MusicRoot`"", "`"$dir`"") -PassThru -NoNewWindow `
        -RedirectStandardOutput (Join-Path $dir 'stdout.txt') -RedirectStandardError (Join-Path $dir 'stderr.txt')
    $null = $proc.Handle  # keeps ExitCode readable after exit
    $marker = Join-Path $dir 'library_ready.json'
    $lines = New-Object System.Collections.Generic.List[string]
    while (-not $proc.HasExited) {
        try {
            $p = Get-Process -Id $proc.Id -ErrorAction Stop
            $w = Get-CimInstance Win32_Process -Filter "ProcessId=$($proc.Id)" -ErrorAction Stop
            $utc = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            $lines.Add(('{0},{1:F0},{2},{3},{4},{5},{6}' -f $utc, $p.TotalProcessorTime.TotalMilliseconds, $p.WorkingSet64,
                $w.ReadTransferCount, $w.WriteTransferCount, $w.ReadOperationCount, $p.Threads.Count))
        } catch { }
        if (Test-Path $marker) { break }
        Start-Sleep -Milliseconds $SampleMs
    }
    $proc.WaitForExit()
    $lines | Add-Content -LiteralPath $samples -Encoding UTF8
    if ($proc.ExitCode -ne 0) { Write-Warning "run $run exit code $($proc.ExitCode); see $dir\stderr.txt" }

    $events = Read-Events (Join-Path $dir 'profile.jsonl')
    $get = { param($name) $events | Where-Object event -eq $name | Select-Object -Last 1 }
    $ready = & $get 'library_ready'
    $tags = & $get 'tags_done'
    $apply = & $get 'db_apply'
    $start = & $get 'process_start'
    $sampleRows = @(Import-Csv -LiteralPath $samples)
    $peakWs = if ($sampleRows.Count) { ($sampleRows | Measure-Object -Property working_set_bytes -Maximum).Maximum } else { 0 }
    $lastSample = if ($sampleRows.Count) { $sampleRows[-1] } else { $null }
    $summaries += [pscustomobject]@{
        run              = $run
        kind             = if ($run -eq 1) { 'first' } else { 'warm' }
        started          = $start.ts_local
        tracks           = $ready.tracks
        total_ms         = [double]$ready.ms
        db_open_ms       = [double](& $get 'db_open').ms
        enumerate_ms     = [double](& $get 'enumerate_done').ms
        compare_ms       = [double](& $get 'compare_done').ms - [double](& $get 'enumerate_done').ms
        tags_ms          = [double]$tags.ms
        tags_parallelism = [double]$tags.effective_parallelism
        file_p50_ms      = [double]$tags.file_p50_ms
        file_p99_ms      = [double]$tags.file_p99_ms
        assemble_ms      = [double](& $get 'assemble_done').ms
        persist_ms       = [double](& $get 'persist_done').ms
        lyrics_ms        = [double](& $get 'lyric_sidecars_done').ms
        first_page_ms    = [double](& $get 'first_page').ms
        peak_ws_mb       = [math]::Round($peakWs / 1MB, 1)
        read_mb          = if ($lastSample) { [math]::Round([double]$lastSample.read_bytes / 1MB, 1) } else { 0 }
        write_mb         = [math]::Round([double]$ready.process_write_bytes / 1MB, 1)
        cpu_ms           = if ($lastSample) { [double]$lastSample.cpu_ms } else { 0 }
        apply            = $apply
        tags             = $tags
        dir              = $dir
    }
    if (-not $KeepDatabases) {
        Get-ChildItem -LiteralPath $dir -Filter 'library.sqlite*' | Remove-Item -Force
    }
}

$summaries | Format-Table run, kind, tracks, total_ms, db_open_ms, enumerate_ms, compare_ms, tags_ms, tags_parallelism,
    file_p50_ms, file_p99_ms, assemble_ms, persist_ms, lyrics_ms, first_page_ms, peak_ws_mb, read_mb, write_mb, cpu_ms -AutoSize

foreach ($s in $summaries) {
    $t = $s.total_ms
    Write-Host ("--- run {0} ({1}) phase share of {2:N0} ms ---" -f $s.run, $s.kind, $t)
    foreach ($phase in 'db_open_ms','enumerate_ms','compare_ms','tags_ms','assemble_ms','persist_ms','lyrics_ms','first_page_ms') {
        Write-Host ("{0,-14} {1,10:N1} ms {2,6:N1}%" -f $phase, $s.$phase, (100 * $s.$phase / [math]::Max($t, 1)))
    }
    if ($s.apply) {
        Write-Host ("db_apply: bulk={6} resolve={0} metadata={1} mapping={2} seen={3} index+fk={7} commit={4} wal={5:N1} MB" -f `
            $s.apply.resolve_track_id_ms, $s.apply.persist_metadata_ms, $s.apply.persist_mapping_ms, $s.apply.seen_insert_ms,
            $s.apply.commit_ms, ($s.apply.wal_bytes / 1MB), $s.apply.bulk, $s.apply.index_rebuild_fk_check_ms)
    }
}
$last = $summaries[-1]
if ($last.tags) {
    Write-Host '--- tag read by extension (last run) ---'
    $last.tags.by_ext | Format-Table ext, files, sum_ms, mean_ms, @{ n = 'MB'; e = { [math]::Round($_.bytes / 1MB, 0) } } -AutoSize
    Write-Host '--- slowest files (last run) ---'
    $last.tags.slowest | Select-Object -First 10 | Format-Table ms, @{ n = 'MB'; e = { [math]::Round($_.bytes / 1MB, 1) } }, path -AutoSize
}
$summaries | Select-Object * -ExcludeProperty apply, tags | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $OutRoot 'summary.json') -Encoding UTF8
Write-Host "output: $OutRoot"
