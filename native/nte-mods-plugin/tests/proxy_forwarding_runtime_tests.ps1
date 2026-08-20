$ErrorActionPreference = "Stop"

$pluginRoot = Split-Path -Parent $PSScriptRoot
$proxyPath = Join-Path $pluginRoot "x64\Release\dwmapi.dll"
if (-not (Test-Path -LiteralPath $proxyPath -PathType Leaf)) {
    throw "Release proxy was not built: $proxyPath"
}

if (-not ("Nte.ProxyForwarding.Native" -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Text;

namespace Nte.ProxyForwarding
{
    public static class Native
    {
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        public static extern IntPtr LoadLibraryW(string path);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        public static extern IntPtr GetModuleHandleW(string name);

        [DllImport("kernel32.dll", CharSet = CharSet.Ansi, SetLastError = true)]
        public static extern IntPtr GetProcAddress(IntPtr module, string name);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        public static extern uint GetModuleFileNameW(
            IntPtr module,
            StringBuilder path,
            uint capacity);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool FreeLibrary(IntPtr module);

        [UnmanagedFunctionPointer(CallingConvention.Winapi)]
        public delegate int DwmFlushDelegate();
    }
}
"@
}

$proxy = [Nte.ProxyForwarding.Native]::LoadLibraryW($proxyPath)
if ($proxy -eq [IntPtr]::Zero) {
    throw "LoadLibraryW(proxy) failed: $([Runtime.InteropServices.Marshal]::GetLastWin32Error())"
}

try {
    $systemDwm = [Nte.ProxyForwarding.Native]::GetModuleHandleW(
        "api-ms-win-dwmapi-l1-1-0.dll")
    if ($systemDwm -eq [IntPtr]::Zero -or $systemDwm -eq $proxy) {
        throw "The API-set anchor did not bind a distinct system DWM module."
    }
    $systemPath = [Text.StringBuilder]::new(32768)
    if ([Nte.ProxyForwarding.Native]::GetModuleFileNameW(
            $systemDwm,
            $systemPath,
            [uint32]$systemPath.Capacity) -eq 0 -or
        -not $systemPath.ToString().StartsWith(
            [Environment]::GetFolderPath("Windows"),
            [StringComparison]::OrdinalIgnoreCase)) {
        throw "The API-set anchor is not backed by the Windows system DWM image."
    }

    $initialize = [Nte.ProxyForwarding.Native]::GetProcAddress(
        $proxy,
        "NteModsPluginInitialize")
    $flushAddress = [Nte.ProxyForwarding.Native]::GetProcAddress(
        $proxy,
        "DwmFlush")
    if ($initialize -eq [IntPtr]::Zero -or $flushAddress -eq [IntPtr]::Zero) {
        throw "The proxy lifecycle/forwarding exports are incomplete."
    }
    $flush = [Runtime.InteropServices.Marshal]::GetDelegateForFunctionPointer(
        $flushAddress,
        [Nte.ProxyForwarding.Native+DwmFlushDelegate])
    $result = $flush.Invoke()
    if ($result -lt 0) {
        throw ("Forwarded DwmFlush failed: 0x{0:X8}" -f ([uint32]$result))
    }

    Write-Output (
        "proxy_forwarding_runtime_tests: PASS system={0} hr=0x{1:X8}" -f
        $systemPath,
        ([uint32]$result))
}
finally {
    if (-not [Nte.ProxyForwarding.Native]::FreeLibrary($proxy)) {
        throw "FreeLibrary(proxy) failed before runtime initialization."
    }
}
