param(
    [ValidateRange(5, 100)]
    [int]$SampleCount = 20,
    [ValidateRange(1, 10000)]
    [int]$Frames = 120,
    [ValidateRange(1, 3600)]
    [int]$TimeoutSeconds = 60,
    [string]$OutputDirectory = "docs/reports/gpu-scene-samples",
    [string]$SampleSetPath = "docs/reports/gpu-scene-sample-set.json"
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$runner = Join-Path $PSScriptRoot "run-gpu-scene-acceptance.ps1"
$cargoPath = Join-Path $env:USERPROFILE ".rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\cargo.exe"
$rustcPath = (Join-Path (Split-Path -Parent $cargoPath) "rustc.exe").Replace("\", "/")
$outputAbsolute = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $OutputDirectory))
$sampleSetAbsolute = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $SampleSetPath))
$repoPrefix = $repoRoot.TrimEnd("\") + "\"

foreach ($path in @($outputAbsolute, $sampleSetAbsolute)) {
    if (-not $path.StartsWith($repoPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "GPU sample output must remain inside the repository: $path"
    }
}
if (Test-Path -LiteralPath $sampleSetAbsolute) {
    throw "Sample-set output already exists and will not be overwritten: $sampleSetAbsolute"
}

Push-Location $repoRoot
try {
    & $cargoPath build --offline --release --config "build.rustc=`"$rustcPath`"" `
        -p genegis-analysis --bin verify_gpu_receipt
    if ($LASTEXITCODE -ne 0) {
        throw "GPU receipt verifier build failed with exit code $LASTEXITCODE"
    }
    New-Item -ItemType Directory -Force -Path $outputAbsolute | Out-Null
    $references = @()
    $identity = $null
    for ($sample = 1; $sample -le $SampleCount; $sample++) {
        $receiptAbsolute = Join-Path $outputAbsolute ("receipt-{0:d3}.json" -f $sample)
        $receiptRelative = [System.IO.Path]::GetRelativePath($repoRoot, $receiptAbsolute).Replace("\", "/")
        & $runner -Release -TimeoutSeconds $TimeoutSeconds -Frames $Frames `
            -ReceiptPath $receiptRelative
        if ($LASTEXITCODE -ne 0) {
            throw "GPU sample $sample failed with exit code $LASTEXITCODE"
        }
        & (Join-Path $repoRoot "target/release/verify_gpu_receipt.exe") $receiptAbsolute | Out-Host
        if ($LASTEXITCODE -ne 0) {
            throw "Persisted GPU sample $sample failed independent verification"
        }
        $receipt = Get-Content -Raw -LiteralPath $receiptAbsolute | ConvertFrom-Json
        $observedIdentity = @(
            $receipt.executable_digest,
            $receipt.build_digest,
            $receipt.copc_digest,
            $receipt.lod1_digest,
            $receipt.os,
            $receipt.cpu,
            $receipt.benchmark.adapter,
            $receipt.benchmark.backend
        ) -join "|"
        if ($null -eq $identity) {
            $identity = $observedIdentity
        }
        elseif ($identity -ne $observedIdentity) {
            throw "GPU artifact, fixture, or hardware identity drifted at sample $sample"
        }
        $references += [ordered]@{
            path = $receiptRelative
            receipt_digest = $receipt.receipt_digest
        }
    }
    $manifest = [ordered]@{
        schema_version = "1.0.0"
        minimum_samples = $SampleCount
        aggregation = [ordered]@{
            first_frame = "nearest_rank_p95"
            steady_state_fps = "minimum"
        }
        receipts = $references
    }
    $bytes = [System.Text.UTF8Encoding]::new($false).GetBytes(
        (($manifest | ConvertTo-Json -Depth 8) + "`n")
    )
    $stream = [System.IO.File]::Open(
        $sampleSetAbsolute,
        [System.IO.FileMode]::CreateNew,
        [System.IO.FileAccess]::Write,
        [System.IO.FileShare]::None
    )
    try {
        $stream.Write($bytes, 0, $bytes.Length)
    }
    finally {
        $stream.Dispose()
    }
    Write-Output $sampleSetAbsolute
}
finally {
    Pop-Location
}
