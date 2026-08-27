param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern("^[A-Za-z0-9_-]+$")]
    [string]$ReviewerCode,
    [Parameter(Mandatory = $true)]
    [ValidatePattern("^[A-Za-z0-9_-]+$")]
    [string]$FacilitatorCode,
    [string]$StudyDirectory = "gate-e-study"
)

$ErrorActionPreference = "Stop"
if ($ReviewerCode -eq $FacilitatorCode) {
    throw "Reviewer and facilitator codes must be distinct."
}
$repoRoot = Split-Path -Parent $PSScriptRoot
$studyRoot = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $StudyDirectory))
$runner = Join-Path $repoRoot "target/release/genegis.exe"
$studyManifest = Join-Path $studyRoot "study-manifest.json"
$gdalBin = Join-Path $repoRoot "tools/pdal/.pixi/envs/default/Library/bin"
$output = Join-Path $studyRoot "session-$ReviewerCode.json"
$runnerDigest = "sha256:$((Get-FileHash -Algorithm SHA256 -LiteralPath $runner).Hash.ToLowerInvariant())"
$previousPath = $env:PATH
if (-not (Test-Path -LiteralPath $studyManifest)) {
    throw "Study is not prepared. Run scripts/prepare-gate-e-study.ps1 first."
}
if (Test-Path -LiteralPath $output) {
    throw "Session output already exists and will not be overwritten: $output"
}
$env:PATH = "$gdalBin;$env:PATH"

Push-Location $repoRoot
try {
    & $runner bench trust-ux --human --study-manifest $studyManifest `
        --runner-digest $runnerDigest `
        --reviewer-code $ReviewerCode `
        --facilitator-code $FacilitatorCode --output $output
    exit $LASTEXITCODE
}
finally {
    $env:PATH = $previousPath
    Pop-Location
}
