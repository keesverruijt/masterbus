# Install the masterbus command-line tools on Windows. In PowerShell:
#
#   irm https://raw.githubusercontent.com/keesverruijt/masterbus/main/install.ps1 | iex
#
# Downloads the latest release and puts masterbus-tui, masterbus-signalk,
# masterbus-set-field and masterbus-dump in %LOCALAPPDATA%\Programs\masterbus,
# which is added to your PATH. No administrator rights needed. Set
# $env:MASTERBUS_VERSION (e.g. v0.4.6) first for a given release.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'  # the progress bar makes the download very slow

$repo = 'keesverruijt/masterbus'
$target = 'x86_64-pc-windows-msvc'
$tools = 'masterbus-tui', 'masterbus-signalk', 'masterbus-set-field', 'masterbus-dump'
$dir = Join-Path $env:LOCALAPPDATA 'Programs\masterbus'

if ($env:MASTERBUS_VERSION) {
    $url = "https://github.com/$repo/releases/download/$env:MASTERBUS_VERSION/masterbus-$target.tar.gz"
} else {
    $url = "https://github.com/$repo/releases/latest/download/masterbus-$target.tar.gz"
}

$tmp = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    Write-Host "Downloading $url"
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri $url -OutFile (Join-Path $tmp 'masterbus.tar.gz') -UseBasicParsing
    # tar.exe ships with Windows 10 (1803) and later.
    tar -xzf (Join-Path $tmp 'masterbus.tar.gz') -C $tmp
    if ($LASTEXITCODE -ne 0) { throw 'unpacking the download failed' }

    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    foreach ($tool in $tools) {
        Copy-Item (Join-Path $tmp "masterbus-$target\$tool.exe") $dir -Force
    }
} finally {
    Remove-Item -Recurse -Force $tmp
}

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $dir) {
    [Environment]::SetEnvironmentVariable('Path', (@($userPath, $dir) | Where-Object { $_ }) -join ';', 'User')
    $env:Path = "$env:Path;$dir"
}

Write-Host ''
Write-Host "Installed $($tools -join ', ') in $dir."
Write-Host 'Start with: masterbus-tui   (in a new window if this one does not find it)'
