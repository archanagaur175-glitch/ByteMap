# ByteMap portable zip (P9.T2).
# Builds the production app + elevated helper, stages them with a README and the
# licences, and writes target/release/bundle/portable/ByteMap-<version>-portable-<arch>.zip.
# Usage: powershell -File scripts/build-portable.ps1

$ErrorActionPreference = 'Stop'

function Run([string]$Name, [scriptblock]$Cmd) {
    Write-Host "==> $Name"
    & $Cmd
    if ($LASTEXITCODE -ne 0) {
        throw "FAILED: $Name (exit code $LASTEXITCODE)"
    }
}

$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
$stageRoot = $null
try {
    $conf = Get-Content -LiteralPath 'src-tauri/tauri.conf.json' -Raw | ConvertFrom-Json
    $version = $conf.version
    if (-not $version) { throw 'tauri.conf.json has no version' }
    $arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64' } else { 'x64' }
    $name = "ByteMap-$version-portable-$arch"

    # --no-bundle still builds the production binary (embedded frontend via
    # custom-protocol) but skips NSIS; the helper is built explicitly because
    # beforeBundleCommand does not run without a bundle step.
    Run 'tauri build --no-bundle' { npx tauri build --no-bundle }
    Run 'cargo build --release -p bytemap-helper' { cargo build --release -p bytemap-helper }

    foreach ($file in @('target/release/bytemap.exe', 'target/release/bytemap-helper.exe')) {
        if (-not (Test-Path -LiteralPath $file)) { throw "missing $file" }
    }

    $stageRoot = Join-Path $root 'target/portable-staging'
    if (Test-Path -LiteralPath $stageRoot) { Remove-Item -LiteralPath $stageRoot -Recurse -Force }
    $stage = Join-Path $stageRoot $name
    New-Item -ItemType Directory -Path $stage -Force | Out-Null

    Copy-Item -LiteralPath 'target/release/bytemap.exe' -Destination $stage
    Copy-Item -LiteralPath 'target/release/bytemap-helper.exe' -Destination $stage
    Copy-Item -LiteralPath 'packaging/README-portable.txt' -Destination (Join-Path $stage 'README.txt')
    Copy-Item -LiteralPath 'LICENSE-MIT' -Destination (Join-Path $stage 'LICENSE-MIT.txt')
    Copy-Item -LiteralPath 'LICENSE-APACHE' -Destination (Join-Path $stage 'LICENSE-APACHE.txt')

    $outDir = Join-Path $root 'target/release/bundle/portable'
    New-Item -ItemType Directory -Path $outDir -Force | Out-Null
    $zip = Join-Path $outDir "$name.zip"
    if (Test-Path -LiteralPath $zip) { Remove-Item -LiteralPath $zip -Force }

    Write-Host "==> compress $name.zip"
    Compress-Archive -LiteralPath $stage -DestinationPath $zip -CompressionLevel Optimal

    $info = Get-Item -LiteralPath $zip
    $hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash
    $count = (Get-ChildItem -LiteralPath $stage -File).Count
    Write-Host ("PORTABLE-ZIP path={0} bytes={1} sha256={2} files={3}" -f $info.FullName, $info.Length, $hash, $count) -ForegroundColor Green
}
finally {
    Pop-Location
    if ($stageRoot -and (Test-Path -LiteralPath $stageRoot)) {
        Remove-Item -LiteralPath $stageRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
