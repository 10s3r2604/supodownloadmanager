# Exercise the actual Windows trust check, including certificate pinning and tampering.
. "$PSScriptRoot/WindowsSigning.ps1"

function Assert-Rejected {
    param([scriptblock] $Action, [string] $ExpectedMessage)
    $rejected = $false
    try { & $Action } catch {
        if ($_.Exception.Message -notlike "*$ExpectedMessage*") { throw }
        $rejected = $true
    }
    if (-not $rejected) { throw "Expected rejection containing: $ExpectedMessage" }
}

$fixture = Join-Path $PSHOME 'pwsh.exe'
$signature = Get-AuthenticodeSignature -LiteralPath $fixture
if ($signature.Status -ne 'Valid' -or $null -eq $signature.TimeStamperCertificate) {
    throw 'The runner PowerShell executable must have a trusted timestamped signature for this test.'
}
$thumbprint = $signature.SignerCertificate.Thumbprint
$info = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($fixture)
$temp = Join-Path ([System.IO.Path]::GetTempPath()) "sdm-signing-check-$([guid]::NewGuid())"
New-Item -ItemType Directory -Path $temp | Out-Null
try {
    Assert-WindowsSignature -Path $fixture -Thumbprint $thumbprint -ProductName $info.ProductName -ProductVersion $info.ProductVersion
    Assert-Rejected { Assert-WindowsSignature -Path $fixture -Thumbprint ('0' * 40) } 'Unexpected signing certificate'
    Assert-Rejected { Assert-WindowsSignature -Path $fixture -Thumbprint $thumbprint -ProductName 'Supo Download Manager' } 'Unexpected product name'
    Assert-Rejected { Assert-WindowsSignature -Path $fixture -Thumbprint $thumbprint -ProductVersion '0.0.0' } 'Unexpected product version'
    $unsigned = Join-Path $temp 'unsigned.exe'
    Set-Content -LiteralPath $unsigned -Value 'This is an unsigned fixture.'
    Assert-Rejected { Assert-WindowsSignature -Path $unsigned -Thumbprint $thumbprint } 'Invalid Authenticode signature'
    Assert-Rejected { Assert-WindowsSignature -Path (Join-Path $temp 'missing.exe') -Thumbprint $thumbprint } 'Missing signed file'
    $tampered = Join-Path $temp 'tampered.exe'
    Copy-Item -LiteralPath $fixture -Destination $tampered
    $bytes = [System.IO.File]::ReadAllBytes($tampered)
    $offset = [int]($bytes.Length / 2)
    $bytes[$offset] = $bytes[$offset] -bxor 1
    [System.IO.File]::WriteAllBytes($tampered, $bytes)
    Assert-Rejected { Assert-WindowsSignature -Path $tampered -Thumbprint $thumbprint } 'Invalid Authenticode signature'
    Assert-Rejected { Assert-SignedWindowsArtifact -Directory $temp -Kind App -Version '0.5.0' -Thumbprint $thumbprint } 'Unexpected signed artifact contents'
    $appDirectory = Join-Path $temp 'app'
    New-Item -ItemType Directory -Path $appDirectory | Out-Null
    Copy-Item -LiteralPath $fixture -Destination (Join-Path $appDirectory 'sdm-desktop.exe')
    Assert-Rejected { Assert-SignedWindowsArtifact -Directory $appDirectory -Kind App -Version '0.5.0' -Thumbprint $thumbprint } 'Unexpected product name'
    $emptyDirectory = Join-Path $temp 'empty'
    New-Item -ItemType Directory -Path $emptyDirectory | Out-Null
    Assert-Rejected { Assert-SignedWindowsArtifact -Directory $emptyDirectory -Kind Installers -Version '0.5.0' -Thumbprint $thumbprint } 'Unexpected signed artifact contents'
    Write-Output '10 Windows signing checks passed.'
} finally {
    Remove-Item -LiteralPath $temp -Recurse -Force
}
