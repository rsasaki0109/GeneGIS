param(
    [string]$StudyDirectory = "gate-e-study"
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$studyRoot = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $StudyDirectory))
$gdalHome = [System.IO.Path]::GetFullPath((Join-Path $repoRoot "tools/pdal/.pixi/envs/default/Library"))
$cargoPath = Join-Path $env:USERPROFILE ".rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\cargo.exe"
$rustcPath = (Join-Path (Split-Path -Parent $cargoPath) "rustc.exe").Replace("\", "/")
$previousGdalHome = $env:GDAL_HOME
$previousGdalVersion = $env:GDAL_VERSION
$previousPath = $env:PATH

$existingHumanSessions = @(
    Get-ChildItem -LiteralPath $studyRoot -Filter "session-*.json" -File -ErrorAction SilentlyContinue |
        Where-Object {
            (Get-Content -Raw -LiteralPath $_.FullName | ConvertFrom-Json).session_kind -eq "human"
        }
)
if ($existingHumanSessions.Count -gt 0) {
    throw "Study preparation is frozen after the first human session; create a new study directory instead."
}

if (-not (Test-Path -LiteralPath (Join-Path $gdalHome "lib/gdal.lib"))) {
    throw "Pinned Pixi GDAL is missing. Run: pixi install --manifest-path tools/pdal/pixi.toml"
}
Copy-Item -LiteralPath (Join-Path $gdalHome "lib/gdal.lib") `
    -Destination (Join-Path $gdalHome "lib/gdal_i.lib") -Force
$env:GDAL_HOME = $gdalHome
$env:GDAL_VERSION = "3.12.3"
$env:PATH = "$(Join-Path $gdalHome 'bin');$env:PATH"

Push-Location $repoRoot
try {
    & $cargoPath build --offline --release --config "build.rustc=`"$rustcPath`"" -p genegis-cli
    if ($LASTEXITCODE -ne 0) {
        throw "Gate E CLI build failed with exit code $LASTEXITCODE"
    }
    New-Item -ItemType Directory -Force -Path $studyRoot | Out-Null
    $binaryPath = Join-Path $repoRoot "target/release/genegis.exe"
    $protocolPath = Join-Path $repoRoot "docs/reports/phase-12-trust-ux-protocol.md"
    $manifest = [ordered]@{
        schema_version = "0.1.0"
        prepared_at = (Get-Date).ToUniversalTime().ToString("o")
        status = "prepared_human_sessions_pending"
        corpus_version = "phase-12-map-first-trust-v1"
        corpus_digest = "sha256:cde790805a1e6dc4f1200d92b5aa95804e94e7604667ab7219af32e5133f88ca"
        build_lock_digest = "sha256:$((Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $repoRoot 'Cargo.lock')).Hash.ToLowerInvariant())"
        runner_digest = "sha256:$((Get-FileHash -Algorithm SHA256 -LiteralPath $binaryPath).Hash.ToLowerInvariant())"
        protocol_digest = "sha256:$((Get-FileHash -Algorithm SHA256 -LiteralPath $protocolPath).Hash.ToLowerInvariant())"
        os = [System.Runtime.InteropServices.RuntimeInformation]::OSDescription
        architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
        terminal_columns = $Host.UI.RawUI.WindowSize.Width
        terminal_rows = $Host.UI.RawUI.WindowSize.Height
        protocol = "docs/reports/phase-12-trust-ux-protocol.md"
        minimum_unique_human_reviewers = 3
        task_count_per_reviewer = 12
        raw_sessions_git_ignored = $true
    }
    $manifest | ConvertTo-Json -Depth 8 | Set-Content -Encoding utf8 `
        -LiteralPath (Join-Path $studyRoot "study-manifest.json")
    Write-Host "Gate E study prepared: $studyRoot"
}
finally {
    $env:GDAL_HOME = $previousGdalHome
    $env:GDAL_VERSION = $previousGdalVersion
    $env:PATH = $previousPath
    Pop-Location
}
