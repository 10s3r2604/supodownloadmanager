# Used by release jobs and Windows signing checks. No signing keys are stored here.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-WindowsSignature {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][ValidatePattern('^[0-9a-fA-F]{40}$')][string] $Thumbprint,
        [string] $ProductName,
        [string] $ProductVersion
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Missing signed file: $Path"
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($signature.Status -ne 'Valid') {
        throw "Invalid Authenticode signature for ${Path}: $($signature.Status)"
    }
    if ($null -eq $signature.SignerCertificate -or $signature.SignerCertificate.Thumbprint -ne $Thumbprint) {
        throw "Unexpected signing certificate for $Path"
    }
    if ($null -eq $signature.TimeStamperCertificate) {
        throw "Missing trusted signing timestamp for $Path"
    }
    $info = [System.Diagnostics.FileVersionInfo]::GetVersionInfo((Resolve-Path -LiteralPath $Path).Path)
    if ($ProductName -and $info.ProductName -ne $ProductName) {
        throw "Unexpected product name for $Path"
    }
    if ($ProductVersion -and $info.ProductVersion -ne $ProductVersion) {
        throw "Unexpected product version for $Path"
    }
}

function Assert-SignedWindowsArtifact {
    param(
        [Parameter(Mandatory)][string] $Directory,
        [Parameter(Mandatory)][ValidateSet('App', 'Installers')][string] $Kind,
        [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+$')][string] $Version,
        [Parameter(Mandatory)][ValidatePattern('^[0-9a-fA-F]{40}$')][string] $Thumbprint
    )

    $expected = @('sdm-desktop.exe')
    if ($Kind -eq 'Installers') {
        $expected = @("Supo.Download.Manager_${Version}_x64-setup.exe", "Supo.Download.Manager_${Version}_x64_en-US.msi")
    }
    $root = (Resolve-Path -LiteralPath $Directory).Path
    $files = @(Get-ChildItem -LiteralPath $root -File -Recurse)
    if ($files.Count -ne $expected.Count) {
        throw "Unexpected signed artifact contents in $Directory"
    }
    foreach ($name in $expected) {
        $path = Join-Path $root $name
        if ([System.IO.Path]::GetExtension($name) -eq '.exe') {
            Assert-WindowsSignature -Path $path -Thumbprint $Thumbprint -ProductName 'Supo Download Manager' -ProductVersion $Version
        } else {
            Assert-WindowsSignature -Path $path -Thumbprint $Thumbprint
        }
    }
}
