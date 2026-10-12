# Report unavailable DLL exports when a Rust test executable fails at startup.
# This does not suppress or replace the failed regression test.
$ErrorActionPreference = 'Stop'
$testBinary = Get-ChildItem target/debug/deps/sdm_desktop-*.exe -ErrorAction SilentlyContinue |
  Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $testBinary) { Write-Host 'No desktop test executable available for loader diagnostics.'; exit 0 }
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$visualStudio = & $vswhere -latest -products '*' -property installationPath
$dumpbin = Get-ChildItem "$visualStudio\VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe" |
  Sort-Object FullName -Descending | Select-Object -First 1
if (-not $dumpbin) { throw 'MSVC dumpbin is unavailable for loader diagnostics.' }
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class SdmLoaderExports {
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  public static extern IntPtr LoadLibraryEx(string file, IntPtr reserved, uint flags);
  [DllImport("kernel32.dll", CharSet=CharSet.Ansi, ExactSpelling=true)]
  public static extern IntPtr GetProcAddress(IntPtr module, string name);
  [DllImport("kernel32.dll", EntryPoint="GetProcAddress", ExactSpelling=true)]
  public static extern IntPtr GetOrdinal(IntPtr module, IntPtr ordinal);
}
'@
$imports = & $dumpbin.FullName /imports $testBinary.FullName
if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the desktop test imports.' }
$dll = ''
$module = [IntPtr]::Zero
$checked = 0
$missing = @()
foreach ($line in $imports) {
  if ($line -match '^\s+(\S+\.dll)\s*$') {
    $dll = $Matches[1]
    # Restrict the probe to the installed system DLLs, which are the source
    # of static Windows imports. No repository DLL is loaded by this helper.
    $module = [SdmLoaderExports]::LoadLibraryEx($dll,[IntPtr]::Zero,0x800)
    if ($module -eq [IntPtr]::Zero) {
      $missing += @{dll=$dll; error=[Runtime.InteropServices.Marshal]::GetLastWin32Error()}
    }
  } elseif ($module -ne [IntPtr]::Zero -and $line -match '\bOrdinal\s+(\d+)\s*$') {
    $checked++
    $ordinal = [int]$Matches[1]
    if ([SdmLoaderExports]::GetOrdinal($module,[IntPtr]$ordinal) -eq [IntPtr]::Zero) {
      $missing += @{dll=$dll; ordinal=$ordinal}
    }
  } elseif ($module -ne [IntPtr]::Zero -and $line -match '^\s+[0-9A-F]+\s+([A-Za-z_?][^\s]*)\s*$') {
    $checked++
    $symbol = $Matches[1]
    if ([SdmLoaderExports]::GetProcAddress($module,$symbol) -eq [IntPtr]::Zero) {
      $missing += @{dll=$dll; symbol=$symbol}
    }
  }
}
Write-Host "Checked $checked imported functions in $($testBinary.Name)."
ConvertTo-Json -InputObject $missing -Depth 5 -Compress | Write-Host
if ($missing.Count -eq 0) { $imports | Write-Host }
