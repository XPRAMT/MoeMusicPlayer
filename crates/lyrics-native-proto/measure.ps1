# Runs the prototype with the given arguments and samples CPU% (dwm, prototype, all msedgewebview2, moemusicplayer)
# and GPU 3D engine utilisation (per pid) with performance counters while it runs. Read-only towards other processes.
param([string]$ProtoArgs = "--windows 3 --mode anim --seconds 12", [int]$Samples = 8)
$exe = Join-Path $PSScriptRoot "target\release\lyrics-native-proto.exe"
$out = [System.IO.Path]::GetTempFileName()
$p = Start-Process -FilePath $exe -ArgumentList $ProtoArgs -PassThru -NoNewWindow -RedirectStandardOutput $out
Start-Sleep -Seconds 2
$cores = [Environment]::ProcessorCount
$pidsOf = @{ dwm = (Get-Process dwm).Id; proto = $p.Id; player = (Get-Process moemusicplayer -ErrorAction SilentlyContinue).Id; webview = (Get-Process msedgewebview2 -ErrorAction SilentlyContinue).Id }
$cpu = @{}; $gpu = @{}
$t0 = @{}; foreach ($k in $pidsOf.Keys) { $t0[$k] = ($pidsOf[$k] | ForEach-Object { (Get-Process -Id $_ -ErrorAction SilentlyContinue).TotalProcessorTime.TotalSeconds } | Measure-Object -Sum).Sum }
$w0 = Get-Date
$g = Get-Counter '\GPU Engine(*engtype_3D)\Utilization Percentage' -SampleInterval 1 -MaxSamples $Samples -ErrorAction SilentlyContinue
$el = ((Get-Date) - $w0).TotalSeconds
foreach ($k in $pidsOf.Keys) {
  $t1 = ($pidsOf[$k] | ForEach-Object { (Get-Process -Id $_ -ErrorAction SilentlyContinue).TotalProcessorTime.TotalSeconds } | Measure-Object -Sum).Sum
  $cpu[$k] = [math]::Round(($t1 - $t0[$k]) / $el * 100, 2)
  $sum = 0
  foreach ($s in $g) { foreach ($c in $s.CounterSamples) { foreach ($id in $pidsOf[$k]) { if ($c.InstanceName -match "pid_$id`_") { $sum += $c.CookedValue } } } }
  $gpu[$k] = [math]::Round($sum / [math]::Max($g.Count,1), 2)
}
$p.WaitForExit()
Get-Content $out | Select-String -Pattern "measured|DWM target|own frames"
"CPU% of one core: " + (($cpu.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join " ")
"GPU 3D %: " + (($gpu.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join " ")
Remove-Item $out
