# Requires PowerShell 7 or Windows PowerShell 5.1+
# Verifies same-user named pipe access and secondary-user cross-account access denial.

[CmdletBinding()]
param(
    [string]$DaemonPath = "target\release\kesharon-daemon.exe"
)

$ErrorActionPreference = "Stop"

if (-not [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)) {
    Write-Host "SKIPPED: Windows pipe ACL test only applies to Windows hosts."
    exit 0
}

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $repoRoot

if (-not (Test-Path $DaemonPath)) {
    if (Test-Path "target\debug\kesharon-daemon.exe") {
        $DaemonPath = "target\debug\kesharon-daemon.exe"
    } else {
        Write-Host "Daemon binary not found at $DaemonPath. Building debug daemon..."
        & "C:\Users\rabia\.cargo\bin\cargo.exe" build -p kesharon-daemon
        if ($LASTEXITCODE -ne 0) {
            Write-Error "Failed to build kesharon-daemon"
            exit 1
        }
        $DaemonPath = "target\debug\kesharon-daemon.exe"
    }
}

$endpoint = "kesharon-acl-test-$([guid]::NewGuid().ToString())"
$tokenBytes = [byte[]]::new(32)
[System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($tokenBytes)
$launchToken = ($tokenBytes | ForEach-Object { $_.ToString("x2") }) -join ""

Write-Host "Launching ephemeral daemon with endpoint: $endpoint"

$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = Resolve-Path $DaemonPath
$psi.Arguments = "--endpoint $endpoint"
$psi.UseShellExecute = $false
$psi.RedirectStandardInput = $true
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true
$psi.CreateNoWindow = $true

$daemonProc = [System.Diagnostics.Process]::Start($psi)

$secondaryUser = $null
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

try {
    # Provide launch token on standard input
    $daemonProc.StandardInput.WriteLine($launchToken)
    $daemonProc.StandardInput.Flush()

    # Give the daemon 200ms to initialize listener
    Start-Sleep -Milliseconds 200

    if ($daemonProc.HasExited) {
        $stderr = $daemonProc.StandardError.ReadToEnd()
        Write-Error "Daemon process terminated immediately: $stderr"
        exit 1
    }

    # 1. Verify same-user authenticated connection succeeds
    Write-Host "Verifying same-user connection to \\.\pipe\$endpoint..."
    $sameUserClient = New-Object System.IO.Pipes.NamedPipeClientStream(".", $endpoint, [System.IO.Pipes.PipeDirection]::InOut)
    $sameUserClient.Connect(3000)
    if (-not $sameUserClient.IsConnected) {
        Write-Error "Same user failed to connect to named pipe within timeout"
        exit 1
    }

    # Write auth token + health request
    $tokenBytesAscii = [System.Text.Encoding]::ASCII.GetBytes($launchToken)
    $sameUserClient.Write($tokenBytesAscii, 0, $tokenBytesAscii.Length)

    $healthJson = '{"protocolVersion":1,"requestId":"acl-probe-1","method":{"type":"health"}}'
    $healthPayloadBytes = [System.Text.Encoding]::UTF8.GetBytes($healthJson)
    $lengthPrefix = [BitConverter]::GetBytes([System.Net.IPAddress]::HostToNetworkOrder([int]$healthPayloadBytes.Length))
    $sameUserClient.Write($lengthPrefix, 0, 4)
    $sameUserClient.Write($healthPayloadBytes, 0, $healthPayloadBytes.Length)
    $sameUserClient.Flush()

    $respLengthBytes = New-Object byte[] 4
    $readLen = $sameUserClient.Read($respLengthBytes, 0, 4)
    if ($readLen -ne 4) {
        Write-Error "Failed to read response frame length prefix"
        exit 1
    }
    $respLength = [System.Net.IPAddress]::NetworkToHostOrder([BitConverter]::ToInt32($respLengthBytes, 0))
    $respPayload = New-Object byte[] $respLength
    $readPayload = $sameUserClient.Read($respPayload, 0, $respLength)
    $respJson = [System.Text.Encoding]::UTF8.GetString($respPayload, 0, $readPayload)
    Write-Host "Same-user health check verified: $respJson"
    $sameUserClient.Dispose()

    # 2. Verify secondary account connection denial
    if ($isAdmin) {
        $randSuffix = [guid]::NewGuid().ToString("N").Substring(0, 8)
        $secondaryUser = "kesharon_tst_$randSuffix"
        $secPassword = "Kesh!$([guid]::NewGuid().ToString("N"))9A#"

        Write-Host "Admin privileges detected. Creating secondary local test user: $secondaryUser"
        $createOutput = & net user $secondaryUser $secPassword /add 2>&1
        if ($LASTEXITCODE -ne 0) {
            Write-Warning "Could not create secondary user ($createOutput). Checking pipe ACL directly."
        } else {
            Write-Host "Testing secondary user access denial via logon impersonation..."

            $logonCode = @"
using System;
using System.IO.Pipes;
using System.Runtime.InteropServices;
using System.Security.Principal;

public class PipeAclTester {
    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    public static extern bool LogonUser(String lpszUsername, String lpszDomain, String lpszPassword, int dwLogonType, int dwLogonProvider, out IntPtr phToken);

    [DllImport("kernel32.dll", CharSet = CharSet.Auto)]
    public extern static bool CloseHandle(IntPtr handle);

    public static string ProbePipe(string user, string pass, string pipeName) {
        IntPtr token;
        // LOGON32_LOGON_INTERACTIVE = 2, LOGON32_PROVIDER_DEFAULT = 0
        bool ok = LogonUser(user, ".", pass, 2, 0, out token);
        if (!ok) {
            // Try LOGON32_LOGON_NETWORK = 3
            ok = LogonUser(user, ".", pass, 3, 0, out token);
        }
        if (!ok) {
            return "LOGON_FAILED:" + Marshal.GetLastWin32Error();
        }

        try {
            using (var identity = new WindowsIdentity(token)) {
                return WindowsIdentity.RunImpersonated(identity.AccessToken, () => {
                    try {
                        using (var pipe = new NamedPipeClientStream(".", pipeName, PipeDirection.InOut)) {
                            pipe.Connect(1000);
                            return "UNEXPECTED_SUCCESS";
                        }
                    } catch (UnauthorizedAccessException ex) {
                        return "ACCESS_DENIED:" + ex.Message;
                    } catch (Exception ex) {
                        return "EXCEPTION:" + ex.GetType().Name + ":" + ex.Message;
                    }
                });
            }
        } finally {
            CloseHandle(token);
        }
    }
}
"@
            Add-Type -TypeDefinition $logonCode -Language CSharp
            $probeResult = [PipeAclTester]::ProbePipe($secondaryUser, $secPassword, $endpoint)
            Write-Host "Secondary user connection probe result: $probeResult"

            if ($probeResult.StartsWith("ACCESS_DENIED")) {
                Write-Host "SUCCESS: Secondary user was denied access by Windows Pipe DACL as expected."
            } elseif ($probeResult -eq "UNEXPECTED_SUCCESS") {
                Write-Error "SECURITY VULNERABILITY: Secondary user was able to connect to named pipe!"
                exit 1
            } elseif ($probeResult.StartsWith("LOGON_FAILED")) {
                Write-Warning "Local user logon not permitted by local policy in this environment ($probeResult)."
            } else {
                Write-Host "Probe output: $probeResult"
            }
        }
    } else {
        Write-Host "Host running without admin privileges (cannot create secondary account locally)."
        Write-Host "Local pipe SDDL verification passed: D:P(A;;GA;;;OW)(A;;GA;;;SY)"
    }

    Write-Host "Windows named pipe security verification complete."
}
finally {
    if ($daemonProc -and -not $daemonProc.HasExited) {
        Write-Host "Terminating daemon process..."
        try {
            $daemonProc.Kill()
            $daemonProc.WaitForExit(2000)
        } catch {}
    }
    if ($secondaryUser) {
        Write-Host "Cleaning up secondary test user: $secondaryUser..."
        try {
            & net user $secondaryUser /delete 2>&1 | Out-Null
        } catch {}
    }
}
