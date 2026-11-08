# Disposable Windows integration probe. Never activates or closes existing windows.
$ErrorActionPreference = 'Stop'
Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class MmvPhaseOneWindows {
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool EnumWindows(Callback callback, IntPtr data);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    delegate bool Callback(IntPtr h, IntPtr data);
    public static int VisibleCount(uint pid) {
        int count = 0;
        EnumWindows((h, _) => { GetWindowThreadProcessId(h, out uint owner); if (owner == pid && IsWindowVisible(h)) count++; return true; }, IntPtr.Zero);
        return count;
    }
}
'@
$exe = Join-Path $PSScriptRoot '../target/debug/examples/phase_one_windows.exe'
if (!(Test-Path $exe)) { throw 'Build first: cargo build --locked --example phase_one_windows -j 4' }
$token = [Guid]::NewGuid().ToString('N')
$temp = Join-Path ([IO.Path]::GetTempPath()) "mmv-phase-one-$token"
$clients = [Collections.Generic.List[Diagnostics.Process]]::new()
$guiIds = [Collections.Generic.HashSet[int]]::new()
$ownedGuiIds = [Collections.Generic.HashSet[int]]::new()
$foreground = [MmvPhaseOneWindows]::GetForegroundWindow()
if ($foreground -eq [IntPtr]::Zero) { throw 'No external foreground window; focus is unverified' }
[uint32]$foregroundPid = 0
[void][MmvPhaseOneWindows]::GetWindowThreadProcessId($foreground, [ref]$foregroundPid)
$focusChanged = $false
$maxVisible = 0
try {
    [void][IO.Directory]::CreateDirectory($temp)
    $file = Join-Path $temp '日本語 memo.md'
    [IO.File]::WriteAllText($file, "# Phase-one probe`n`nLoaded by the Pipe worker.")
    # Start together without waiting for any GUI/response. One missing file must
    # produce an error response bearing its own request ID, not a startup success.
    foreach ($id in 1..4) {
        $info = [Diagnostics.ProcessStartInfo]::new($exe)
        $info.UseShellExecute = $false
        $info.CreateNoWindow = $true
        $info.RedirectStandardOutput = $true
        $info.RedirectStandardError = $true
        foreach ($arg in @('open', $token, '4', "request-$id", $(if ($id -eq 3) { Join-Path $temp 'missing.md' } else { $file }))) {
            $info.ArgumentList.Add($arg)
        }
        $clients.Add([Diagnostics.Process]::Start($info))
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while (@($clients | Where-Object { !$_.HasExited }).Count -gt 0 -or
           @($ownedGuiIds | Where-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue }).Count -gt 0) {
        if ([MmvPhaseOneWindows]::GetForegroundWindow() -ne $foreground) { $focusChanged = $true }
        # Only discover descendants launched for this unique run, never other GUIs.
        $children = Get-CimInstance Win32_Process -Filter "Name = 'phase_one_windows.exe'" |
            Where-Object { $_.CommandLine -like "*gui $token*" -and $_.ParentProcessId -in $clients.Id }
        foreach ($child in $children) {
            [void]$ownedGuiIds.Add([int]$child.ProcessId)
            $maxVisible = [Math]::Max($maxVisible, [MmvPhaseOneWindows]::VisibleCount($child.ProcessId))
        }
        if ([DateTime]::UtcNow -ge $deadline) { throw 'Probe timed out' }
        Start-Sleep -Milliseconds 10
    }
    foreach ($client in $clients) {
        $output = $client.StandardOutput.ReadToEnd().Trim()
        $errorOutput = $client.StandardError.ReadToEnd()
        if ($client.ExitCode -ne 0) { throw "CLI failed: $errorOutput" }
        $parts = $output.Split('|')
        if ($parts.Count -ne 4 -or $parts[3] -ne '1') { throw "Unexpected GUI reply: $output" }
        $expectedStatus = if ($parts[0] -eq 'request-3') { 'error' } else { 'ok' }
        if ($parts[1] -ne $expectedStatus) { throw "Wrong read result: $output" }
        [void]$guiIds.Add([int]$parts[2])
        Write-Output $output
    }
    if ($guiIds.Count -ne 1 -or $ownedGuiIds.Count -ne 1) { throw 'Concurrent starts did not converge to one observed GUI process' }
    if ($maxVisible -ne 1) { throw "Expected one actual visible window; observed maximum $maxVisible" }
    if ($focusChanged -or [MmvPhaseOneWindows]::GetForegroundWindow() -ne $foreground) {
        throw 'Foreground changed during probe; non-activation not confirmed (external interaction is also possible)'
    }
    foreach ($guiId in $guiIds) {
        $process = Get-Process -Id $guiId -ErrorAction SilentlyContinue
        if ($process -and !$process.WaitForExit(5000)) { throw 'GUI did not exit after its probe requests' }
    }
    Write-Output "PASS: 4 request IDs/read results, one GUI PID/window, external foreground HWND=$foreground PID=$foregroundPid unchanged in sampled observations."
} finally {
    $clientIds = @($clients | ForEach-Object { $_.Id })
    foreach ($client in $clients) { if (!$client.HasExited) { $client.Kill() }; $client.Dispose() }
    # Cleanup only GUI descendants belonging to this exact invocation.
    $children = Get-CimInstance Win32_Process -Filter "Name = 'phase_one_windows.exe'" |
        Where-Object { $_.CommandLine -like "*gui $token*" -and $_.ParentProcessId -in $clientIds }
    foreach ($child in $children) { Stop-Process -Id $child.ProcessId -ErrorAction SilentlyContinue }
    if (Test-Path $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
}
