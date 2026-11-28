# Build the Windows MSI installer for quack from an already-built quack.exe.
#
#   pwsh -File scripts/package-msi.ps1 -Target x86_64-pc-windows-msvc -Version v0.2.0
#
# Requires the WiX v5 CLI (`dotnet tool install --global wix`).
# Produces `dist/quack-<target>.msi` containing `quack.exe`, LICENSE, NOTICE,
# PATH registration, and the editable `defaults\` bundle.
#
# Signing is optional: if `$env:WINDOWS_SIGN_CERT_PFX` is set (base64-encoded PFX
# plus `$env:WINDOWS_SIGN_CERT_PASSWORD`), the MSI is signed with signtool.exe;
# otherwise an unsigned MSI is produced without requiring external accounts.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc')]
    [string]$Target,

    [Parameter(Mandatory = $true)]
    [string]$Version,

    [string]$Binary,
    [string]$OutDir,
    [switch]$Plucked
)

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
if (-not $Binary) {
    $Binary = Join-Path $repoRoot "target\$Target\release\quack.exe"
}
if (-not $OutDir) {
    $OutDir = Join-Path $repoRoot 'dist'
}
if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) {
    throw "package-msi.ps1: no binary at $Binary"
}
if (-not (Get-Command wix -ErrorAction SilentlyContinue)) {
    throw "package-msi.ps1: `wix` CLI not found on PATH (install with: dotnet tool install --global wix)"
}

# MSI ProductVersion requires 3-4 numeric components (major.minor.patch[.build]).
$clean = $Version.TrimStart('v', 'V') -replace '[-+].*$', ''
$parts = @($clean -split '\.' | Where-Object { $_ -match '^\d+$' })
while ($parts.Count -lt 3) { $parts += '0' }
if ($parts.Count -gt 4) { $parts = $parts[0..3] }
$msiVersion = $parts -join '.'

$arch = if ($Target -like 'aarch64-*') { 'arm64' } else { 'x64' }

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$outPath = Join-Path (Resolve-Path $OutDir).Path "quack-$Target.msi"

$stage = Join-Path ([System.IO.Path]::GetTempPath()) ("quack-msi-" + [guid]::NewGuid().ToString('N'))
$defaultsStage = Join-Path $stage 'defaults'
$binStage = Join-Path $stage 'bin'
$objStage = Join-Path $stage 'obj'

function Test-HostedProvider([System.IO.FileInfo]$file) {
    $m = Select-String -Path $file.FullName -Pattern '^base_url\s*=\s*"[a-z]*://([^/:"]*)' | Select-Object -First 1
    $host_ = if ($m) { $m.Matches[0].Groups[1].Value } else { '' }
    return -not ($host_ -like '127.*' -or $host_ -eq 'localhost' -or $host_ -eq '[::1]')
}

try {
    New-Item -ItemType Directory -Force -Path $defaultsStage, $binStage, $objStage | Out-Null
    Copy-Item -LiteralPath $Binary -Destination (Join-Path $binStage 'quack.exe')

    # Stage defaults/ cleanly without dotfiles or editor backup files.
    $srcDefaults = Join-Path $repoRoot 'defaults'
    Get-ChildItem -LiteralPath $srcDefaults -Recurse -File | ForEach-Object {
        if ($_.Name.StartsWith('.') -or $_.Name.EndsWith('~')) { return }
        $rel = $_.FullName.Substring($srcDefaults.Length).TrimStart('\', '/')
        $dest = Join-Path $defaultsStage $rel
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $dest) | Out-Null
        Copy-Item -LiteralPath $_.FullName -Destination $dest
    }

    if ($Plucked) {
        Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $defaultsStage 'config.toml')
        $provDir = Join-Path $defaultsStage 'providers.d'
        if (Test-Path -LiteralPath $provDir -PathType Container) {
            Get-ChildItem -LiteralPath $provDir -Filter '*.toml' | ForEach-Object {
                if (Test-HostedProvider $_) { Remove-Item -Force $_.FullName }
            }
        }
    }

    $wxs = Join-Path $repoRoot 'packaging\windows\quack.wxs'
    $wixArgs = @(
        'build', $wxs,
        '-arch', $arch,
        '-d', "Version=$msiVersion",
        '-bindpath', "Bin=$binStage",
        '-bindpath', "Repo=$repoRoot",
        '-bindpath', "Defaults=$defaultsStage",
        '-intermediateFolder', $objStage,
        '-o', $outPath
    )
    Write-Host "quack: building $outPath (Version=$msiVersion, arch=$arch)"
    & wix @wixArgs
    if ($LASTEXITCODE -ne 0) {
        throw "wix build failed with exit code $LASTEXITCODE"
    }

    if ($env:WINDOWS_SIGN_CERT_PFX) {
        $signtool = Get-Command signtool.exe -ErrorAction SilentlyContinue
        if (-not $signtool) {
            throw "WINDOWS_SIGN_CERT_PFX is set, but signtool.exe was not found on PATH"
        }
        $pfxPath = Join-Path $stage 'cert.pfx'
        [System.IO.File]::WriteAllBytes($pfxPath, [Convert]::FromBase64String($env:WINDOWS_SIGN_CERT_PFX))
        $tsUrl = if ($env:WINDOWS_SIGN_TIMESTAMP_URL) { $env:WINDOWS_SIGN_TIMESTAMP_URL } else { 'http://timestamp.digicert.com' }
        $signArgs = @('sign', '/fd', 'SHA256', '/tr', $tsUrl, '/td', 'SHA256', '/f', $pfxPath)
        if ($env:WINDOWS_SIGN_CERT_PASSWORD) {
            $signArgs += @('/p', $env:WINDOWS_SIGN_CERT_PASSWORD)
        }
        $signArgs += $outPath
        & $signtool.Source @signArgs
        if ($LASTEXITCODE -ne 0) {
            throw "signtool failed with exit code $LASTEXITCODE"
        }
    }

    Write-Output $outPath
} finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $stage
}
