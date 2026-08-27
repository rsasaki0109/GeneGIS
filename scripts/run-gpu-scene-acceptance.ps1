param(
    [ValidateRange(1, 3600)]
    [int]$TimeoutSeconds = 180,
    [ValidateRange(1, 10000)]
    [int]$Frames = 120,
    [string]$ManifestPath = "examples/nagoya-population-density/data/nagoya-scene-fixture-manifest.json",
    [string]$ReceiptPath = "docs/reports/phase-14-m1-gpu-hardware-receipt.json",
    [switch]$Release
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$cargoPath = Join-Path $env:USERPROFILE ".rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\cargo.exe"
$rustcPath = (Join-Path (Split-Path -Parent $cargoPath) "rustc.exe").Replace("\", "/")
$manifestAbsolute = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $ManifestPath))
$receiptAbsolute = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $ReceiptPath))
$buildDirectory = if ($Release) { "release" } else { "debug" }
$binaryPath = Join-Path $repoRoot "target\$buildDirectory\gpu_scene_acceptance.exe"
$previousBuildProfile = $env:GENEGIS_BUILD_PROFILE

if (-not (Test-Path -LiteralPath $cargoPath)) {
    throw "stable cargo executable not found at $cargoPath"
}
if (-not (Test-Path -LiteralPath $manifestAbsolute)) {
    throw "fixture manifest not found at $manifestAbsolute"
}
if (Test-Path -LiteralPath $receiptAbsolute) {
    throw "Receipt output already exists and will not be overwritten: $receiptAbsolute"
}

Push-Location $repoRoot
try {
    if ($Release) {
        & $cargoPath build --offline --release --config "build.rustc=`"$rustcPath`"" -p genegis-analysis --bin gpu_scene_acceptance
    }
    else {
        & $cargoPath build --offline --config "build.rustc=`"$rustcPath`"" -p genegis-analysis --bin gpu_scene_acceptance
    }
    if ($LASTEXITCODE -ne 0) {
        throw "GPU acceptance binary build failed with exit code $LASTEXITCODE"
    }

    $receiptParent = Split-Path -Parent $receiptAbsolute
    New-Item -ItemType Directory -Force -Path $receiptParent | Out-Null
    $runId = [Guid]::NewGuid().ToString("N")
    $stdoutPath = Join-Path $env:TEMP "genegis-gpu-$runId.stdout.log"
    $stderrPath = Join-Path $env:TEMP "genegis-gpu-$runId.stderr.log"
    $env:GENEGIS_BUILD_PROFILE = $buildDirectory
    $process = Start-Process -FilePath $binaryPath `
        -ArgumentList @($manifestAbsolute, $receiptAbsolute, $Frames) `
        -WorkingDirectory $repoRoot `
        -WindowStyle Hidden `
        -RedirectStandardOutput $stdoutPath `
        -RedirectStandardError $stderrPath `
        -PassThru

    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        $process.Kill($true)
        $process.WaitForExit()
        [Console]::Error.WriteLine("GPU acceptance timed out after $TimeoutSeconds seconds; no receipt was admitted.")
        exit 124
    }

    $stdout = if (Test-Path -LiteralPath $stdoutPath) { Get-Content -Raw -LiteralPath $stdoutPath } else { "" }
    $stderr = if (Test-Path -LiteralPath $stderrPath) { Get-Content -Raw -LiteralPath $stderrPath } else { "" }
    if ($stdout) { Write-Host $stdout.TrimEnd() }
    if ($stderr) { [Console]::Error.WriteLine($stderr.TrimEnd()) }
    if ($process.ExitCode -ne 0) {
        if (Test-Path -LiteralPath $receiptAbsolute) {
            [Console]::Error.WriteLine("GPU acceptance process exited with code $($process.ExitCode); a non-passing receipt was retained at $receiptAbsolute.")
        }
        else {
            [Console]::Error.WriteLine("GPU acceptance process exited with code $($process.ExitCode); no receipt was admitted.")
        }
        exit $process.ExitCode
    }
    if (-not (Test-Path -LiteralPath $receiptAbsolute)) {
        throw "GPU acceptance exited successfully without a receipt"
    }
    Write-Host "Accepted receipt: $receiptAbsolute"
}
finally {
    Pop-Location
    $env:GENEGIS_BUILD_PROFILE = $previousBuildProfile
    if ($stdoutPath -and (Test-Path -LiteralPath $stdoutPath)) {
        Remove-Item -LiteralPath $stdoutPath -Force
    }
    if ($stderrPath -and (Test-Path -LiteralPath $stderrPath)) {
        Remove-Item -LiteralPath $stderrPath -Force
    }
}
