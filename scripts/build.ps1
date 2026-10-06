$ErrorActionPreference = 'Stop'
$env:PATH = "$(Join-Path $env:USERPROFILE '.cargo\bin');$env:PATH"
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    & cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Build failed.' }
    Write-Host "Executable: $(Join-Path (Get-Location) 'target\release\woomy.exe')"
} finally { Pop-Location }
