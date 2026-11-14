# ByteMap phase gate check.
# Runs all checks in the owner-approved order, stops at the first failure with exit code 1.
# Usage: powershell -File scripts/check.ps1

function Run([string]$Name, [scriptblock]$Cmd) {
    Write-Host "==> $Name"
    & $Cmd
    if ($LASTEXITCODE -ne 0) {
        Write-Host "FAILED: $Name (exit code $LASTEXITCODE)" -ForegroundColor Red
        exit 1
    }
}

Run 'cargo fmt --all -- --check' { cargo fmt --all -- --check }
Run 'cargo check --workspace'    { cargo check --workspace }
Run 'cargo clippy -D warnings'   { cargo clippy --workspace --all-targets -- -D warnings }
Run 'cargo test --workspace'     { cargo test --workspace }
Run 'npm run build'              { npm run build }
Run 'npx vitest run'             { npx vitest run --passWithNoTests }

Write-Host 'ALL CHECKS PASSED' -ForegroundColor Green
exit 0
