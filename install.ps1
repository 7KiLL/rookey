# Install rookey on Windows: the release build, into %LOCALAPPDATA%\rookey, on your PATH.
# Nothing else is downloaded: `rookey setup` asks for a speech model, or an ElevenLabs key instead.
#
#   irm https://rookey.click/install.ps1 | iex
#
# $env:ROOKEY_VERSION = "v0.1.0" picks a release (default: the latest), $env:ROOKEY_BUILD = "cpu"
# skips the CUDA build.
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"  # the progress bar makes Invoke-WebRequest crawl

$repo = "7KiLL/rookey"
$dir = Join-Path $env:LOCALAPPDATA "rookey"

if (-not [Environment]::Is64BitOperatingSystem) { throw "rookey: there is only a 64-bit build." }
$build = "x86_64-windows"
# the CUDA build brings its own CUDA runtime; it only needs the NVIDIA driver
$nvidia = Get-CimInstance Win32_VideoController | Where-Object { $_.Name -match "NVIDIA" }
if ($env:ROOKEY_BUILD -ne "cpu" -and $nvidia) { $build = "x86_64-windows-cuda" }

$url = if ($env:ROOKEY_VERSION) {
    "https://github.com/$repo/releases/download/$($env:ROOKEY_VERSION)/rookey-$build.zip"
} else {
    "https://github.com/$repo/releases/latest/download/rookey-$build.zip"
}

$zip = Join-Path ([IO.Path]::GetTempPath()) "rookey-$build.zip"
Write-Host "downloading rookey-$build"
Invoke-WebRequest -Uri $url -OutFile $zip
# a running listener holds rookey.exe open; it starts again from the page or at the next login
Get-Process rookey -ErrorAction SilentlyContinue | Stop-Process -Force
New-Item -ItemType Directory -Force -Path $dir | Out-Null
Expand-Archive -Path $zip -DestinationPath $dir -Force
Remove-Item $zip
Write-Host "installed $dir\rookey.exe"

$path = [Environment]::GetEnvironmentVariable("Path", "User")
if (($path -split ";") -notcontains $dir) {
    [Environment]::SetEnvironmentVariable("Path", ((@($path, $dir) | Where-Object { $_ }) -join ";"), "User")
    $env:Path += ";$dir"
    Write-Host "added $dir to your PATH (new terminals pick it up)"
}
Write-Host ""
Write-Host "next: rookey setup   (checks your mic, then a speech model or an ElevenLabs key)"
