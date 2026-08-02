param()

$ErrorActionPreference = 'Stop'
$artifact = $PSScriptRoot
$workspace = (Resolve-Path (Join-Path $artifact '..\..')).Path
$id = Get-Date -Format 'yyyyMMdd-HHmmssfff'
$validation = Join-Path $artifact ("delivery-validation-" + $id)
$patchRoot = Join-Path $validation 'patch-root'
$rollbackRoot = Join-Path $validation 'rollback-root'
$log = Join-Path $artifact 'delivery-verification.log'

New-Item -ItemType Directory -Force -Path $patchRoot, $rollbackRoot | Out-Null
Copy-Item -Path (Join-Path $artifact 'original\*') -Destination $patchRoot -Recurse -Force
Copy-Item -Path (Join-Path $artifact 'modified-snapshot\*') -Destination $rollbackRoot -Recurse -Force

& {
    Write-Output 'COMMAND: git apply --check --ignore-space-change --ignore-whitespace --whitespace=nowarn parity-fixes.patch'
    Push-Location $patchRoot
    try {
        git init --quiet
        git apply --check --ignore-space-change --ignore-whitespace --whitespace=nowarn (Join-Path $artifact 'parity-fixes.patch')
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    } finally {
        Pop-Location
    }
    Write-Output 'OUTPUT: patch_check=passed'
    Write-Output 'EXIT_STATUS: 0'

    Write-Output 'COMMAND: git apply --ignore-space-change --ignore-whitespace --whitespace=nowarn parity-fixes.patch'
    Push-Location $patchRoot
    try {
        git apply --ignore-space-change --ignore-whitespace --whitespace=nowarn (Join-Path $artifact 'parity-fixes.patch')
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    } finally {
        Pop-Location
    }
    Write-Output 'OUTPUT: patch_apply=passed'
    Write-Output 'EXIT_STATUS: 0'

    Write-Output 'COMMAND: python verify_text_equivalence.py PATCH_ROOT modified-snapshot scoped-files.txt'
    python (Join-Path $artifact 'verify_text_equivalence.py') $patchRoot (Join-Path $artifact 'modified-snapshot') (Join-Path $artifact 'scoped-files.txt')
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Output 'EXIT_STATUS: 0'

    Write-Output 'COMMAND: python behavior_probe.py PATCH_ROOT'
    python (Join-Path $artifact 'behavior_probe.py') $patchRoot
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Output 'EXIT_STATUS: 0'

    Write-Output 'COMMAND: powershell -File rollback.ps1 -Root ROLLBACK_ROOT'
    & (Join-Path $artifact 'rollback.ps1') -Root $rollbackRoot
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Output 'EXIT_STATUS: 0'

    $original = Get-Content -Raw -LiteralPath (Join-Path $artifact 'original-hashes.json') | ConvertFrom-Json -AsHashtable
    foreach ($relative in $original.Keys) {
        $target = Join-Path $rollbackRoot $relative
        if ($null -eq $original[$relative]) {
            if (Test-Path -LiteralPath $target) { throw "expected absent after rollback: $relative" }
            continue
        }
        $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $target).Hash.ToLowerInvariant()
        if ($actual -ne $original[$relative]) { throw "original hash mismatch: $relative" }
    }
    Write-Output ("OUTPUT: rollback_reopened_verified_files={0}" -f $original.Count)

    Write-Output 'COMMAND: python verify_hashes.py WORKSPACE modified-hashes.json'
    python (Join-Path $artifact 'verify_hashes.py') $workspace (Join-Path $artifact 'modified-hashes.json')
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Output 'EXIT_STATUS: 0'

    Write-Output 'COMMAND: reopen original-hashes.json modified-hashes.json parity-fixes.patch verification inputs rollback.ps1'
    foreach ($file in @('original-hashes.json', 'modified-hashes.json', 'parity-fixes.patch', 'baseline-behavior.json', 'modified-behavior.json', 'rollback.ps1')) {
        $path = Join-Path $artifact $file
        if (-not (Test-Path -LiteralPath $path -PathType Leaf) -or (Get-Item -LiteralPath $path).Length -eq 0) {
            throw "artifact missing or empty: $file"
        }
    }
    Write-Output 'OUTPUT: delivery_roles_reopened=6'
    Write-Output 'EXIT_STATUS: 0'
    Write-Output ("VALIDATION_ROOT: {0}" -f $validation)
} 2>&1 | Tee-Object -FilePath $log
