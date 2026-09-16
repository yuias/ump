<#
.SYNOPSIS
    Downloads DotGothic16-Regular.ttf (SIL OFL 1.1) from google/fonts, pinned
    to a fixed commit, and installs it into ump's font directory. ump does
    not bundle fonts; it auto-detects this file there when [font].path is
    unset in settings.toml (see README.md's Fonts section).
.PARAMETER Dest
    Target font directory. Defaults to %LOCALAPPDATA%\ump\fonts, next to
    settings.toml (matches Config::fonts_dir() in src/config.rs). Override for
    testing.
#>
param(
    [string]$Dest = (Join-Path $env:LOCALAPPDATA 'ump\fonts')
)

Set-StrictMode -Version 2
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$CommitSha = '9a6cc6b8ce992aff77b69c857c46af0b42cdff76'
$TtfSha256 = '3AD9AF88726D42B40F7F365F0DCAC785AF73CF20EA6F1D5B44E57CC21150B8F1'
$TtfUrl = "https://raw.githubusercontent.com/google/fonts/$CommitSha/ofl/dotgothic16/DotGothic16-Regular.ttf"
$OflUrl = "https://raw.githubusercontent.com/google/fonts/$CommitSha/ofl/dotgothic16/OFL.txt"

New-Item -ItemType Directory -Path $Dest -Force | Out-Null
$Ttf = Join-Path $Dest 'DotGothic16-Regular.ttf'
$License = Join-Path $Dest 'DotGothic16-OFL.txt'

function Get-Sha256($Path) {
    # Use .NET directly rather than Get-FileHash: avoids depending on
    # Microsoft.PowerShell.Utility module auto-loading being enabled.
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        $stream = [System.IO.File]::OpenRead($Path)
        try {
            $hashBytes = $sha256.ComputeHash($stream)
        }
        finally {
            $stream.Dispose()
        }
    }
    finally {
        $sha256.Dispose()
    }
    return ([BitConverter]::ToString($hashBytes) -replace '-', '')
}

if ((Test-Path $Ttf) -and ((Get-Sha256 $Ttf) -eq $TtfSha256)) {
    Write-Host "Already installed and verified: $Ttf"
    exit 0
}

# Download next to the destination so the final move is a same-volume,
# effectively atomic rename -- a failed download never leaves $Ttf partial.
$Tmp = Join-Path $Dest ("DotGothic16-Regular.ttf." + [System.IO.Path]::GetRandomFileName())
try {
    Invoke-WebRequest -Uri $TtfUrl -OutFile $Tmp -UseBasicParsing

    $ActualSha256 = Get-Sha256 $Tmp
    if ($ActualSha256 -ne $TtfSha256) {
        Write-Error "Checksum mismatch: expected $TtfSha256, got $ActualSha256"
        exit 1
    }

    Move-Item -Path $Tmp -Destination $Ttf -Force
    Invoke-WebRequest -Uri $OflUrl -OutFile $License -UseBasicParsing
}
finally {
    if (Test-Path $Tmp) {
        Remove-Item -Path $Tmp -Force
    }
}

Write-Host "Installed: $Ttf"
Write-Host "License:   $License"
Write-Host "ump picks this up automatically on next launch when [font].path is unset in settings.toml."
