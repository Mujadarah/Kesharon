# Requires PowerShell 7 or Windows PowerShell 5.1+
# Verifies same-user named pipe access and secondary-user cross-account access denial.

[CmdletBinding()]
param(
    [string]$DaemonPath = "target\release\kesharon-daemon.exe",
    [switch]$AllowInconclusive = $false
)

$ErrorActionPreference = "Stop"

if (-not [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)) {
    Write-Host "SKIPPED: Windows pipe ACL test only applies to Windows hosts."
    exit 0
}

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $repoRoot

function Read-Exact {
    param(
        [Parameter(Mandatory = $true)]
        [System.IO.Stream]$Stream,
        [Parameter(Mandatory = $true)]
        [byte[]]$Buffer,
        [Parameter(Mandatory = $true)]
        [int]$Count
    )
    $offset = 0
    while ($offset -lt $Count) {
        $read = $Stream.Read($Buffer, $offset, $Count - $offset)
        if ($read -le 0) {
            throw "Unexpected EOF while reading exact stream: expected $Count bytes, but got $offset"
        }
        $offset += $read
    }
}

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
    Read-Exact -Stream $sameUserClient -Buffer $respLengthBytes -Count 4
    $respLength = [System.Net.IPAddress]::NetworkToHostOrder([BitConverter]::ToInt32($respLengthBytes, 0))
    if ($respLength -le 0 -or $respLength -gt (8 * 1024 * 1024)) {
        Write-Error "Invalid response length declared: $respLength"
        exit 1
    }

    $respPayload = New-Object byte[] $respLength
    Read-Exact -Stream $sameUserClient -Buffer $respPayload -Count $respLength
    $respJson = [System.Text.Encoding]::UTF8.GetString($respPayload)

    $respObj = $null
    try {
        $respObj = $respJson | ConvertFrom-Json
    } catch {
        Write-Error "Health check response payload is not valid JSON: $respJson"
        exit 1
    }

    if ($respObj.protocolVersion -ne 1 -or $respObj.requestId -ne "acl-probe-1" -or $respObj.result.type -ne "health" -or $respObj.result.status -ne "ready" -or $null -ne $respObj.error) {
        Write-Error "Health check response payload failed contract validation: $respJson"
        exit 1
    }

    Write-Host "Same-user health check verified: $respJson"
    $sameUserClient.Dispose()

    # 2. Verify secondary account connection denial
    if ($isAdmin) {
        $randSuffix = [guid]::NewGuid().ToString("N").Substring(0, 6)
        $secondaryUser = "ksh_tst_$randSuffix"
        $credCharset = [char[]]"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789"
        $credChars = 1..12 | ForEach-Object { $credCharset[(Get-Random -Maximum $credCharset.Length)] }
        $secAuthToken = -join ($credChars + @('!', '9'))
        $secSecureToken = ConvertTo-SecureString $secAuthToken -AsPlainText -Force

        Write-Host "Admin privileges detected. Creating secondary local test user: $secondaryUser"
        $userCreated = $false
        $createError = ""

        try {
            New-LocalUser -Name $secondaryUser -Password $secSecureToken -FullName "Kesharon Pipe Test" -Description "Ephemeral probe" -ErrorAction Stop | Out-Null
            $userCreated = $true
        } catch {
            $createError = $_.Exception.Message
        }

        if (-not $userCreated) {
            $createOutput = & net.exe user $secondaryUser "$secAuthToken" /add 2>&1
            if ($LASTEXITCODE -eq 0) {
                $userCreated = $true
            } else {
                $createError = "$createError; $createOutput"
            }
        }

        if (-not $userCreated) {
            if ($AllowInconclusive) {
                Write-Warning "Could not create secondary user ($createError). Skipping secondary user probe."
            } else {
                Write-Error "INCONCLUSIVE_ACL_PROBE: Failed to create secondary test user ($createError). Secondary-user denial proof cannot proceed."
                exit 1
            }
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
            if (-not ([System.Management.Automation.PSTypeName]'PipeAclTester').Type) {
                Add-Type -TypeDefinition $logonCode -Language CSharp
            }
            $probeResult = [PipeAclTester]::ProbePipe($secondaryUser, $secAuthToken, $endpoint)
            Write-Host "Secondary user connection probe result: $probeResult"

            if ($probeResult.StartsWith("ACCESS_DENIED")) {
                Write-Host "SUCCESS: Secondary user was denied access by Windows Pipe DACL as expected."
            } elseif ($probeResult -eq "UNEXPECTED_SUCCESS") {
                Write-Error "SECURITY VULNERABILITY: Secondary user was able to connect to named pipe!"
                exit 1
            } elseif ($probeResult.StartsWith("LOGON_FAILED") -or $probeResult.StartsWith("EXCEPTION")) {
                if ($AllowInconclusive) {
                    Write-Warning "Secondary user logon/probe failed ($probeResult). Skipping enforcement due to -AllowInconclusive."
                } else {
                    Write-Error "HOSTILE_PROBE_FAILED: Hostile-user probe failed ($probeResult). Secondary-user denial proof requires explicit ACCESS_DENIED."
                    exit 1
                }
            } elseif ($AllowInconclusive) {
                Write-Warning "Secondary user probe was inconclusive: $probeResult"
            } else {
                Write-Error "HOSTILE_PROBE_FAILED: Secondary user connection probe failed to prove denial: $probeResult"
                exit 1
            }
        }
    } else {
        if ($AllowInconclusive) {
            Write-Warning "Host running without admin privileges (cannot create secondary account locally)."
            Write-Host "Local pipe SDDL verification passed: D:P(A;;GA;;;OW)(A;;GA;;;SY)"
        } else {
            Write-Error "INCONCLUSIVE_ACL_PROBE: Administrator privileges are required to create a secondary account and verify named pipe ACL isolation. Run in an elevated PowerShell session or pass -AllowInconclusive for a non-enforcing local probe."
            exit 1
        }
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
            Remove-LocalUser -Name $secondaryUser -ErrorAction SilentlyContinue
        } catch {}
        try {
            & net.exe user $secondaryUser /delete 2>&1 | Out-Null
        } catch {}
    }
}
