param(
    [string]$StudyDirectory = "gate-e-study",
    [string]$OutputPath = "gate-e-study/aggregate.json"
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$studyRoot = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $StudyDirectory))
$aggregatePath = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $OutputPath))
$runner = Join-Path $repoRoot "target/release/genegis.exe"
$studyManifest = Join-Path $studyRoot "study-manifest.json"
$gdalBin = Join-Path $repoRoot "tools/pdal/.pixi/envs/default/Library/bin"
$previousPath = $env:PATH
$sessions = @(Get-ChildItem -LiteralPath $studyRoot -Filter "session-*.json" -File | Sort-Object Name)
$runnerDigest = "sha256:$((Get-FileHash -Algorithm SHA256 -LiteralPath $runner).Hash.ToLowerInvariant())"
if ($sessions.Count -eq 0) {
    throw "No Gate E session files found in $studyRoot"
}
$arguments = @(
    "bench", "trust-ux-aggregate",
    "--study-manifest", $studyManifest,
    "--runner-digest", $runnerDigest
)
foreach ($session in $sessions) {
    $arguments += @("--input", $session.FullName)
}
$arguments += @("--output", $aggregatePath)
$env:PATH = "$gdalBin;$env:PATH"

Push-Location $repoRoot
try {
    & $runner @arguments
    exit $LASTEXITCODE
}
finally {
    $env:PATH = $previousPath
    Pop-Location
}
