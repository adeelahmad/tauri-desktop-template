# {{app_name}} for Windows in one command (PowerShell):
#
#   irm https://raw.githubusercontent.com/{{github_repo}}/main/scripts/install.ps1 | iex
#
# Downloads the newest -setup.exe from the GitHub releases and installs it for the current
# user (no admin rights). The app is not code-signed, so SmartScreen may ask once.
$ErrorActionPreference = 'Stop'
$repo = '{{github_repo}}'
Write-Host 'Finding the newest build...'
$releases = Invoke-RestMethod "https://api.github.com/repos/$repo/releases?per_page=30"
$asset = $releases | ForEach-Object { $_.assets } | Where-Object { $_.name -like '*-setup.exe' -and $_.name -notlike 'update-*' } | Select-Object -First 1
if (-not $asset) { throw "No Windows build found at https://github.com/$repo/releases" }
$file = Join-Path $env:TEMP $asset.name
Write-Host "Downloading $($asset.name)..."
Invoke-WebRequest $asset.browser_download_url -OutFile $file
Unblock-File $file
Start-Process $file -ArgumentList '/S' -Wait
Write-Host 'Installed. Find {{app_name}} in the Start menu.'
