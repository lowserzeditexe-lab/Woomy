param([switch]$Install)
$ErrorActionPreference = 'Stop'
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (Test-Path -LiteralPath $cargoBin) { $env:PATH = "$cargoBin;$env:PATH" }
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$hasVc = $false
if (Test-Path -LiteralPath $vswhere) { $hasVc = [bool](& $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath) }
if ($Install) {
    if (-not (Get-Command winget -ErrorAction SilentlyContinue)) { throw 'Install Microsoft App Installer (winget) first.' }
    if (-not $hasVc) {
        & winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --silent --accept-package-agreements --accept-source-agreements --disable-interactivity --override '--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
        if ($LASTEXITCODE -ne 0) { throw 'Visual C++ Build Tools installation failed.' }
    }
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        & winget install --id Rustlang.Rustup --exact --silent --accept-package-agreements --accept-source-agreements --disable-interactivity
        if ($LASTEXITCODE -ne 0) { throw 'Rustup installation failed.' }
        $env:PATH = "$cargoBin;$env:PATH"
    }
    & rustup component add rustfmt clippy
    if ($LASTEXITCODE -ne 0) { throw 'Rust component installation failed.' }
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { throw 'Rust not found. Run scripts/setup.ps1 -Install.' }
& cargo --version
& rustc --version
if (Test-Path -LiteralPath $vswhere) { & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath }
Write-Host 'Development tools only. No Wii U content is downloaded. Build with scripts/build.ps1.'
