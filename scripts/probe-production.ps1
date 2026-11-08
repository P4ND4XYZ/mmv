# Product probe. Never activates a window or terminates an existing process.
# Run from PowerShell 7 after: cargo build --locked --bin mmv -j 4
$ErrorActionPreference = 'Stop'
$exe = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../target/debug/mmv.exe'))
if (!(Test-Path -LiteralPath $exe)) { throw 'Build mmv.exe first' }
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class MmvProductWindows {
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
function Get-Guis {
    @(Get-CimInstance Win32_Process -Filter "Name = 'mmv.exe'" |
        Where-Object { $_.CommandLine -match '(?:^|\s)--mmv-internal-gui(?:\s|$)' })
}
if (@(Get-Guis).Count -gt 0) {
    Write-Output 'UNVERIFIED: existing mmv GUI detected; probe stopped without touching it.'
    exit 2
}
$foreground = [MmvProductWindows]::GetForegroundWindow()
if ($foreground -eq [IntPtr]::Zero) { throw 'UNVERIFIED: no external foreground HWND' }
[uint32]$foregroundPid = 0
[void][MmvProductWindows]::GetWindowThreadProcessId($foreground, [ref]$foregroundPid)
$clients = [Collections.Generic.List[Diagnostics.Process]]::new()
$guis = [Collections.Generic.Dictionary[int,Diagnostics.Process]]::new()
$temp = Join-Path ([IO.Path]::GetTempPath()) ('mmv-product-' + [Guid]::NewGuid().ToString('N'))
$samples = 0
$focusChanged = $false
$maxWindows = 0
function Sample {
    $script:samples++
    if ([MmvProductWindows]::GetForegroundWindow() -ne $foreground) { $script:focusChanged = $true }
    # Ownership requires this exact executable, internal argument, and an observed
    # parent CLI from this run. No arbitrary instance-selection flag/env is used.
    foreach ($child in (Get-Guis)) {
        if ($child.ExecutablePath -ne $exe -or $guis.ContainsKey([int]$child.ProcessId)) { continue }
        $parent = @($clients | Where-Object { $_.Id -eq $child.ParentProcessId -and $_.StartTime.ToUniversalTime() -le $child.CreationDate.ToUniversalTime() })
        if ($parent.Count -eq 0) { throw 'UNVERIFIED: unrelated GUI appeared during probe; refusing to operate on it' }
        $process = [Diagnostics.Process]::GetProcessById([int]$child.ProcessId)
        if ([Math]::Abs(($process.StartTime.ToUniversalTime() - $child.CreationDate.ToUniversalTime()).TotalSeconds) -gt 1) { throw 'GUI PID identity changed' }
        # Open/cache a process handle while alive so ExitCode remains available
        # after normal shutdown (GetProcessById alone does not retain one).
        $null = $process.Handle
        $guis.Add($process.Id, $process)
    }
    $visible = 0
    foreach ($gui in $guis.Values) {
        if (!$gui.HasExited) { $visible += [MmvProductWindows]::VisibleCount([uint32]$gui.Id) }
    }
    $script:maxWindows = [Math]::Max($maxWindows, $visible)
    if ($visible -gt 1) { throw "More than one visible GUI window: $visible" }
}
function Start-Cli([string[]]$Arguments, [string]$WorkingDirectory = $temp, [switch]$SkipSample) {
    if (!$SkipSample) { Sample }
    $info = [Diagnostics.ProcessStartInfo]::new($exe)
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.WorkingDirectory = $WorkingDirectory
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    foreach ($arg in $Arguments) { $info.ArgumentList.Add($arg) }
    $process = [Diagnostics.Process]::Start($info)
    $clients.Add($process)
    # Read asynchronously so even verbose output cannot block process exit.
    [pscustomobject]@{ Process = $process; Out = $process.StandardOutput.ReadToEndAsync(); Err = $process.StandardError.ReadToEndAsync() }
}
function Wait-Cli($Client, [int]$Code, [bool]$InfoOutput = $false) {
    $deadline = [DateTime]::UtcNow.AddSeconds(40)
    while (!$Client.Process.HasExited) {
        Sample
        if ([DateTime]::UtcNow -ge $deadline) { throw 'CLI deadline exceeded' }
        Start-Sleep -Milliseconds 10
    }
    Sample
    if (!$Client.Out.Wait(2000) -or !$Client.Err.Wait(2000)) { throw 'CLI exited but redirected streams remained open' }
    $stdout = $Client.Out.GetAwaiter().GetResult()
    $stderr = $Client.Err.GetAwaiter().GetResult()
    if ($Client.Process.ExitCode -ne $Code) { throw "CLI exit $($Client.Process.ExitCode), expected ${Code}: $stderr" }
    if ($Code -eq 0) {
        if ($stderr.Length -ne 0 -or (($stdout.Length -gt 0) -ne $InfoOutput)) { throw 'Wrong success output streams' }
    } elseif ($stderr.Length -eq 0 -or $stdout.Length -ne 0) { throw 'Wrong failure output streams' }
}
function Assert-OneGui {
    Sample
    $live = @($guis.Values | Where-Object { !$_.HasExited })
    if ($live.Count -ne 1 -or [MmvProductWindows]::VisibleCount([uint32]$live[0].Id) -ne 1) { throw 'Expected exactly one owned GUI process and visible window' }
    return $live[0]
}
function Close-OwnedGui($Gui) {
    # Normal WM_CLOSE only to this probe's exact child, never a user window.
    if (!$guis.ContainsKey($Gui.Id) -or $Gui.HasExited) { throw 'GUI ownership/lifetime mismatch' }
    if (!$Gui.CloseMainWindow()) { throw 'Could not request normal GUI close' }
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while (!$Gui.HasExited) {
        Sample
        if ([DateTime]::UtcNow -ge $deadline) { throw 'GUI normal shutdown timed out' }
        Start-Sleep -Milliseconds 10
    }
    $Gui.WaitForExit()
    if ($Gui.ExitCode -ne 0) { throw "GUI abnormal exit: $($Gui.ExitCode)" }
}
try {
    [void][IO.Directory]::CreateDirectory($temp)
    $other = Join-Path $temp '別 cwd'
    [void][IO.Directory]::CreateDirectory($other)
    $file = Join-Path $temp '日本語 memo.md'
    [IO.File]::WriteAllText($file, "# Product probe`n`n" + ("edit paragraph`n`n" * 100))
    [IO.File]::WriteAllText((Join-Path $other '別 file.md'), '# Other working directory')
    foreach ($arguments in @(@('--help'), @('--version'), @('open', '--help'))) {
        Wait-Cli (Start-Cli $arguments) 0 $true
    }
    Wait-Cli (Start-Cli @()) 2
    foreach ($arguments in @(@('open'), @('scroll', 'sideways'), @('--unknown'))) {
        Wait-Cli (Start-Cli $arguments) 2
    }
    foreach ($command in @('top', 'bottom', 'back', 'forward', 'scroll')) {
        $arguments = if ($command -eq 'scroll') { @('scroll', 'down') } else { @($command) }
        Wait-Cli (Start-Cli $arguments) 1
    }
    Sample
    if ($guis.Count -ne 0) { throw 'Information/usage/navigation started a GUI' }
    Write-Output 'PASS: help/version/usage and missing-GUI navigation do not start GUI'

    Wait-Cli (Start-Cli @('open', 'missing.md')) 1
    $first = Assert-OneGui
    Wait-Cli (Start-Cli @('日本語 memo.md')) 0
    Wait-Cli (Start-Cli @('open', '別 file.md') $other) 0
    Wait-Cli (Start-Cli @('open', 'missing.md') $other) 1
    foreach ($arguments in @(@('back'), @('forward'), @('top'), @('bottom'), @('scroll', 'down'), @('scroll', 'half-up'), @('scroll', 'page-down'))) {
        Wait-Cli (Start-Cli $arguments) 0
    }
    if ((Assert-OneGui).Id -ne $first.Id) { throw 'Existing GUI was replaced' }
    Close-OwnedGui $first
    Write-Output "PASS: initial read failure, subsequent success, existing GUI/other cwd/Japanese paths, normal exit PID=$($first.Id)"

    $batch = @()
    foreach ($id in 1..4) {
        $arguments = if ($id -eq 3) { @('open', 'missing.md') } else { @('open', '日本語 memo.md') }
        $batch += [pscustomobject]@{ Client = (Start-Cli $arguments -SkipSample); Code = $(if ($id -eq 3) { 1 } else { 0 }) }
    }
    foreach ($item in $batch) { Wait-Cli $item.Client $item.Code }
    $second = Assert-OneGui
    if ($guis.Count -ne 2 -or $second.Id -eq $first.Id) { throw 'Concurrent initial open did not converge to one new GUI' }
    Close-OwnedGui $second
    Wait-Cli (Start-Cli @('open', $file)) 0
    $third = Assert-OneGui
    if ($guis.Count -ne 3) { throw 'Restart after normal shutdown not observed' }
    Close-OwnedGui $third
    if ($focusChanged) { throw 'Foreground HWND changed in sampled observations; non-activation not confirmed (external interaction also possible)' }
    Write-Output "PASS: concurrent opens converge to one GUI/window; normal shutdown/restart; maximum visible windows=$maxWindows"
    Write-Output 'PASS: real CLI exit and stdout/stderr EOF while its independent GUI remains alive.'
    Write-Output "PASS: foreground HWND=$foreground PID=$foregroundPid unchanged in $samples sampled observations (10ms sleeps plus process enumeration time)."
    Write-Output 'UNVERIFIED: physical IME behavior and visual focus inspection; unsampled foreground transitions are not excluded.'
} finally {
    # Process objects retain identity: never kill by a name, reused PID or a tree.
    foreach ($client in $clients) { if (!$client.HasExited) { $client.Kill(); $client.WaitForExit() }; $client.Dispose() }
    foreach ($gui in $guis.Values) { if (!$gui.HasExited) { $gui.Kill(); $gui.WaitForExit() }; $gui.Dispose() }
    if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
}
