$ErrorActionPreference = 'Stop'
$env:PATH = "$(Join-Path $env:USERPROFILE '.cargo\bin');$env:PATH"
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    & cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw 'Formatting check failed.' }
    & cargo clippy --workspace --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Clippy failed.' }
    & cargo test --workspace
    if ($LASTEXITCODE -ne 0) { throw 'Tests failed.' }
} finally { Pop-Location }
