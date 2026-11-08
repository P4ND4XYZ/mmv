# Probe the real GUI main wiring with isolated scratch/log directories.
# Requires PowerShell 7 and a built target/debug/mmv.exe. Never activates windows.
$ErrorActionPreference = 'Stop'
$exe = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../target/debug/mmv.exe'))
if (!(Test-Path -LiteralPath $exe)) { throw 'Build mmv.exe first' }
function Get-Guis {
    @(Get-CimInstance Win32_Process -Filter "Name = 'mmv.exe'" |
        Where-Object { $_.CommandLine -match '(?:^|\s)--mmv-internal-gui(?:\s|$)' })
}
if (@(Get-Guis).Count) { Write-Output 'UNVERIFIED: existing GUI; stopped without touching it'; exit 2 }
$fixture = Join-Path ([IO.Path]::GetTempPath()) ('mmv-main-probe-' + [Guid]::NewGuid().ToString('N'))
$cli = $null
$gui = $null
try {
    $temp = Join-Path $fixture 'temp'
    $local = Join-Path $fixture 'local'
    $emptyPath = Join-Path $fixture 'empty-path'
    foreach ($dir in @($temp, $local, $emptyPath)) { [void][IO.Directory]::CreateDirectory($dir) }
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $root = Join-Path $temp "my-markdown-viewer-mermaid-v1-$sid"
    $stale = Join-Path $root 'synthetic-stale'
    $unmarked = Join-Path $root 'unmarked'
    [void][IO.Directory]::CreateDirectory($stale)
    [void][IO.Directory]::CreateDirectory($unmarked)
    [IO.File]::WriteAllText((Join-Path $stale 'owner.lease'), "my-markdown-viewer Mermaid scratch v1`n", [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $stale 'input.mmd'), 'synthetic leftover')
    [IO.File]::WriteAllText((Join-Path $unmarked 'keep.txt'), 'keep')
    $file = Join-Path $fixture 'probe.md'
    $source = "# Main probe`n`n``````mermaid`nflowchart TD`nA --> B`n```````n"
    [IO.File]::WriteAllText($file, $source, [Text.UTF8Encoding]::new($false))
    $info = [Diagnostics.ProcessStartInfo]::new($exe)
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.Environment['TEMP'] = $temp
    $info.Environment['TMP'] = $temp
    $info.Environment['LOCALAPPDATA'] = $local
    # Deterministic real generation failure, without executing a fake mmdc.
    $info.Environment['PATH'] = $emptyPath
    $info.ArgumentList.Add('open')
    $info.ArgumentList.Add($file)
    $cli = [Diagnostics.Process]::Start($info)
    $stdout = $cli.StandardOutput.ReadToEndAsync()
    $stderr = $cli.StandardError.ReadToEndAsync()
    $deadline = [DateTime]::UtcNow.AddSeconds(35)
    $log = Join-Path $local 'my-markdown-viewer/logs/viewer.log'
    while ($true) {
        foreach ($child in (Get-Guis)) {
            if ($null -ne $gui -and $child.ProcessId -eq $gui.Id) { continue }
            if ($child.ExecutablePath -ne $exe -or $child.ParentProcessId -ne $cli.Id -or
                $child.CreationDate.ToUniversalTime() -lt $cli.StartTime.ToUniversalTime()) {
                throw 'UNVERIFIED: unrelated GUI appeared; refusing to touch it'
            }
            if ($null -ne $gui) { throw 'Unexpected second GUI' }
            $candidate = [Diagnostics.Process]::GetProcessById([int]$child.ProcessId)
            if ([Math]::Abs(($candidate.StartTime.ToUniversalTime() - $child.CreationDate.ToUniversalTime()).TotalSeconds) -gt 1) { $candidate.Dispose(); throw 'GUI PID identity changed' }
            $null = $candidate.Handle
            $gui = $candidate
        }
        if ($null -ne $gui -and $gui.HasExited) { throw 'GUI exited before observations' }
        if ($null -ne $gui -and $cli.HasExited -and !(Test-Path -LiteralPath $stale) -and (Test-Path -LiteralPath $log)) { break }
        if ([DateTime]::UtcNow -ge $deadline) { throw 'UNVERIFIED: deadline waiting for GUI, cleanup, CLI or log' }
        Start-Sleep -Milliseconds 20
    }
    if (!$stdout.Wait(2000) -or !$stderr.Wait(2000)) { throw 'CLI streams did not reach EOF' }
    if ($cli.ExitCode -ne 0 -or $stdout.Result.Length -or $stderr.Result.Length) { throw "CLI failed: $($stderr.Result)" }
    if (!(Test-Path -LiteralPath (Join-Path $unmarked 'keep.txt'))) { throw 'Unmarked directory was removed' }
    $entry = [IO.File]::ReadAllText($log)
    if (!$entry.Contains($file.Replace('\', '\\')) -or !$entry.Contains('utf8_byte_offset=14') -or !$entry.Contains('mmdc not found on PATH') -or $entry.Contains('flowchart TD')) {
        throw "Unexpected log entry: $entry"
    }
    Write-Output 'PASS: real GUI startup removed a synthetic marked stale scratch directory and preserved an unmarked directory.'
    Write-Output 'PASS: real GUI diagram request with isolated empty PATH logged mmdc-not-found to isolated LOCALAPPDATA, without diagram source.'
    if (!$gui.CloseMainWindow()) { throw 'Could not request owned GUI normal close' }
    if (!$gui.WaitForExit(10000)) { throw 'Owned GUI normal shutdown timed out' }
    if ($gui.ExitCode -ne 0) { throw "GUI abnormal exit: $($gui.ExitCode)" }
    Write-Output "PASS: owned GUI PID=$($gui.Id) exited normally. The stale fixture was synthetic, not a real GUI crash."
} finally {
    if ($null -ne $cli) { if (!$cli.HasExited) { $cli.Kill(); $cli.WaitForExit() }; $cli.Dispose() }
    if ($null -ne $gui) {
        if (!$gui.HasExited) {
            if ($gui.CloseMainWindow()) { [void]$gui.WaitForExit(3000) }
            if (!$gui.HasExited) { $gui.Kill(); $gui.WaitForExit() }
        }
        $gui.Dispose()
    }
    if (Test-Path -LiteralPath $fixture) { Remove-Item -LiteralPath $fixture -Recurse -Force }
}
