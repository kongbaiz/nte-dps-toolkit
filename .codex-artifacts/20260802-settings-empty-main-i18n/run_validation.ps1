param()

$ErrorActionPreference = 'Continue'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$logs = Join-Path $PSScriptRoot 'validation-logs'
New-Item -ItemType Directory -Force -Path $logs | Out-Null

$commands = @(
    @{ Id = 'root-fmt'; Command = 'cargo fmt --check' },
    @{ Id = 'root-check'; Command = 'cargo check' },
    @{ Id = 'root-test'; Command = 'cargo test' },
    @{ Id = 'gui-check'; Command = 'cargo check --bin nte-dps-tool --features gui' },
    @{ Id = 'gui-test'; Command = 'cargo test --features gui' },
    @{ Id = 'gui-clippy'; Command = 'cargo clippy --bin nte-dps-tool --features gui -- -D warnings' },
    @{ Id = 'cli-check'; Command = 'cargo check --bin nte-core --no-default-features --features cli' },
    @{ Id = 'cli-test'; Command = 'cargo test --no-default-features --features cli' },
    @{ Id = 'cli-clippy'; Command = 'cargo clippy --bin nte-core --no-default-features --features cli -- -D warnings' },
    @{ Id = 'cli-tree'; Command = 'cargo tree -e normal --no-default-features --features cli' },
    @{ Id = 'tauri-fmt'; Command = 'cargo fmt --check --manifest-path src-tauri/Cargo.toml' },
    @{ Id = 'tauri-check'; Command = 'cargo check --manifest-path src-tauri/Cargo.toml' },
    @{ Id = 'tauri-focused-test'; Command = 'cargo test --manifest-path src-tauri/Cargo.toml settings' },
    @{ Id = 'tauri-test'; Command = 'cargo test --manifest-path src-tauri/Cargo.toml' },
    @{ Id = 'tauri-clippy'; Command = 'cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings' },
    @{ Id = 'frontend-focused-test'; Command = 'pnpm --dir frontend test -- src/lib/tauri/settings-client.test.ts src/lib/tauri/settings-contract.test.ts src/features/empty-curtain/equipment-canvas-layout.test.ts src/features/main-dps/main-dps-model.test.ts src/lib/i18n.test.ts' },
    @{ Id = 'frontend-typecheck'; Command = 'pnpm --dir frontend typecheck' },
    @{ Id = 'frontend-lint'; Command = 'pnpm --dir frontend lint' },
    @{ Id = 'frontend-test'; Command = 'pnpm --dir frontend test' },
    @{ Id = 'frontend-build'; Command = 'pnpm --dir frontend build' },
    @{ Id = 'frontend-prettier'; Command = 'pnpm --dir frontend exec prettier --check src/features/settings/settings-page.tsx src/features/settings/settings-catalog.tsx src/features/settings/use-settings.ts src/lib/tauri/settings-client.ts src/lib/tauri/settings-client.test.ts src/lib/tauri/settings-contract.ts src/lib/tauri/settings-contract.test.ts src/features/empty-curtain/empty-curtain-page.tsx src/features/empty-curtain/equipment-canvas-grid.tsx src/features/empty-curtain/equipment-canvas-layout.ts src/features/empty-curtain/equipment-canvas-layout.test.ts src/features/main-dps/main-dps-page.tsx src/features/main-dps/main-dps-model.ts src/features/main-dps/main-dps-model.test.ts src/lib/i18n.test.ts' },
    @{ Id = 'i18n-audit'; Command = 'python .codex-artifacts/20260802-settings-empty-main-i18n/audit_production_i18n.py' },
    @{ Id = 'diff-check'; Command = 'git diff --check' }
)

$results = @()
Push-Location $root
try {
    foreach ($entry in $commands) {
        $started = Get-Date
        $log = Join-Path $logs ($entry.Id + '.log')
        Write-Output ("=== {0}: {1}" -f $entry.Id, $entry.Command)
        Invoke-Expression ($entry.Command + ' 2>&1') | Tee-Object -FilePath $log
        $status = $LASTEXITCODE
        $results += [ordered]@{
            id = $entry.Id
            command = $entry.Command
            exitStatus = $status
            elapsedSeconds = [math]::Round(((Get-Date) - $started).TotalSeconds, 3)
            log = $log
        }
        Write-Output ("=== {0}: exit {1}" -f $entry.Id, $status)
    }
} finally {
    Pop-Location
}

$treeLog = Join-Path $logs 'cli-tree.log'
$forbidden = @('tauri ', 'wry ', 'webview2-com ', 'eframe ', 'egui ', 'wgpu ', 'rfd ', 'raw-window-handle ')
$treeText = Get-Content -Raw -LiteralPath $treeLog
$hits = @($forbidden | Where-Object { $treeText -match ('(?m)^.*' + [regex]::Escape($_)) })
$treeAuditLog = Join-Path $logs 'cli-tree-audit.log'
("FORBIDDEN_GUI_DEPS={0}" -f $hits.Count) | Set-Content -Encoding utf8 -LiteralPath $treeAuditLog
$results += [ordered]@{
    id = 'cli-tree-audit'
    command = 'inspect cli-tree.log for tauri/wry/webview2-com/eframe/egui/wgpu/rfd/raw-window-handle'
    exitStatus = $(if ($hits.Count -eq 0) { 0 } else { 1 })
    elapsedSeconds = 0
    log = $treeAuditLog
}

$results | ConvertTo-Json -Depth 4 | Set-Content -Encoding utf8 -LiteralPath (Join-Path $PSScriptRoot 'validation-results.json')
$failed = @($results | Where-Object { $_.exitStatus -ne 0 })
Write-Output ("validation_commands={0}" -f $results.Count)
Write-Output ("validation_failures={0}" -f $failed.Count)
if ($failed.Count -ne 0) {
    exit 1
}
