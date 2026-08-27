param(
    [Parameter(Mandatory = $true)]
    [string]$SourceUrl,
    [Parameter(Mandatory = $true)]
    [string]$GpuSampleSetPath,
    [string]$RangeReceiptPath = "docs/reports/horizon-4-h4-6-managed-cloud-range-receipt.json",
    [string]$MatrixReceiptPath = "docs/reports/horizon-4-h4-6-managed-cloud-receipt.json"
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$profileRelative = "benchmarks/profiles/managed-cloud-nagoya.json"
$profileAbsolute = Join-Path $repoRoot $profileRelative
$sampleSetAbsolute = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $GpuSampleSetPath))
$rangeAbsolute = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $RangeReceiptPath))
$matrixAbsolute = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $MatrixReceiptPath))
$repoPrefix = $repoRoot.TrimEnd("\") + "\"
$gdalHome = [System.IO.Path]::GetFullPath((Join-Path $repoRoot "tools/pdal/.pixi/envs/default/Library"))
$cargoPath = Join-Path $env:USERPROFILE ".rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\cargo.exe"
$rustcPath = (Join-Path (Split-Path -Parent $cargoPath) "rustc.exe").Replace("\", "/")
$sourceUri = [System.Uri]$SourceUrl
$previousAllowedHosts = $env:GENEGIS_REMOTE_ALLOWED_HOSTS
$previousGdalHome = $env:GDAL_HOME
$previousGdalVersion = $env:GDAL_VERSION
$previousPath = $env:PATH

foreach ($path in @($sampleSetAbsolute, $rangeAbsolute, $matrixAbsolute)) {
    if (-not $path.StartsWith($repoPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Managed-cloud evidence must remain inside the repository: $path"
    }
}
if (-not $sourceUri.IsAbsoluteUri -or $sourceUri.Scheme -notin @("http", "https")) {
    throw "SourceUrl must be an absolute HTTP(S) URL"
}
if ($sourceUri.UserInfo) {
    throw "SourceUrl must not embed credentials"
}
if (-not (Test-Path -LiteralPath $sampleSetAbsolute)) {
    throw "Managed-cloud GPU sample set is missing: $sampleSetAbsolute"
}
if (-not (Test-Path -LiteralPath (Join-Path $gdalHome "lib/gdal_i.lib"))) {
    throw "Pinned Pixi GDAL is missing. Run: pixi install --manifest-path tools/pdal/pixi.toml"
}

Push-Location $repoRoot
try {
    $env:GDAL_HOME = $gdalHome
    $env:GDAL_VERSION = "3.12.3"
    $env:PATH = "$(Join-Path $gdalHome 'bin');$env:PATH"
    $env:GENEGIS_REMOTE_ALLOWED_HOSTS = $sourceUri.DnsSafeHost
    & $cargoPath build --offline --release --config "build.rustc=`"$rustcPath`"" `
        -p genegis-testkit --bin managed_cloud_range_acceptance `
        --bin performance_matrix_acceptance --bin verify_performance_matrix_receipt
    if ($LASTEXITCODE -ne 0) {
        throw "Managed-cloud acceptance binaries failed to build with exit code $LASTEXITCODE"
    }
    & (Join-Path $repoRoot "target/release/managed_cloud_range_acceptance.exe") `
        $profileAbsolute $SourceUrl $rangeAbsolute
    if ($LASTEXITCODE -ne 0) {
        throw "Managed-cloud HTTP Range evidence failed with exit code $LASTEXITCODE"
    }
    & (Join-Path $repoRoot "target/release/performance_matrix_acceptance.exe") `
        $profileAbsolute $sampleSetAbsolute $matrixAbsolute $rangeAbsolute
    if ($LASTEXITCODE -ne 0) {
        throw "Managed-cloud performance matrix did not pass (exit code $LASTEXITCODE)"
    }
    & (Join-Path $repoRoot "target/release/verify_performance_matrix_receipt.exe") `
        $profileAbsolute $matrixAbsolute
    if ($LASTEXITCODE -ne 0) {
        throw "Persisted managed-cloud performance matrix failed independent verification (exit code $LASTEXITCODE)"
    }
    Write-Output $matrixAbsolute
}
finally {
    if ($null -eq $previousAllowedHosts) {
        Remove-Item Env:GENEGIS_REMOTE_ALLOWED_HOSTS -ErrorAction SilentlyContinue
    }
    else {
        $env:GENEGIS_REMOTE_ALLOWED_HOSTS = $previousAllowedHosts
    }
    $env:GDAL_HOME = $previousGdalHome
    $env:GDAL_VERSION = $previousGdalVersion
    $env:PATH = $previousPath
    Pop-Location
}
