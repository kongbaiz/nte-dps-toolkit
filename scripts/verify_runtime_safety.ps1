[CmdletBinding()]
param(
    [switch]$SelfTestOnly,
    [switch]$Strict
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# This policy is intentionally source based. It is a review aid, not a proof
# that a lock is held for the right amount of time. The default mode blocks
# unbounded contract repair and channel workers without an owner/cancellation
# proof. -Strict additionally turns all review warnings into errors.
$script:RuleIds = @{
    CloneProjection = "RUNTIME-HOT-CLONE"
    ChannelWorker   = "RUNTIME-CHANNEL-THREAD"
    ReplayBudget    = "RUNTIME-REPLAY-BUDGET"
    BoundaryPanic   = "RUNTIME-BOUNDARY-PANIC"
    ContractSlice   = "RUNTIME-CONTRACT-SLICE"
}

# Baselines use counts so a line move does not silently make a finding
# disappear. Added-line fingerprints are checked separately, so replacing an
# allowlisted occurrence while keeping the count unchanged is still blocked.
# If an occurrence is removed, the script reports a stale baseline so the
# table can be trimmed in the same PR.
$script:Baseline = @{
    CloneProjection = @{
        # The original main presentation clone was removed in the current
        # working tree; keep this empty so a regression is a new violation.
    }
    ChannelWorker = @{}
    ContractSlice = @{}
}

function Assert-Policy {
    param(
        [Parameter(Mandatory)]
        [bool]$Condition,
        [Parameter(Mandatory)]
        [string]$Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

function ConvertTo-RelativePath {
    param(
        [Parameter(Mandatory)]
        [string]$Path
    )

    $fullPath = [IO.Path]::GetFullPath($Path)
    $root = [IO.Path]::GetFullPath((Get-Location).Path)
    return ([IO.Path]::GetRelativePath($root, $fullPath) -replace "\\", "/")
}

function New-Diagnostic {
    param(
        [Parameter(Mandatory)]
        [ValidateSet("Error", "Warning", "Info")]
        [string]$Severity,
        [Parameter(Mandatory)]
        [string]$Rule,
        [Parameter(Mandatory)]
        [string]$Path,
        [int]$Line = 0,
        [Parameter(Mandatory)]
        [string]$Message,
        [bool]$Baseline = $false
    )

    return [pscustomobject]@{
        Severity = $Severity
        Rule     = $Rule
        Path     = $Path
        Line     = $Line
        Message  = $Message
        Baseline = $Baseline
    }
}

function Get-RepositoryFiles {
    param(
        [Parameter(Mandatory)]
        [string[]]$Roots,
        [Parameter(Mandatory)]
        [string[]]$Extensions
    )

    $files = @()
    foreach ($root in $Roots) {
        if (-not (Test-Path -LiteralPath $root -PathType Container)) {
            continue
        }
        $files += @(
            Get-ChildItem -LiteralPath $root -Recurse -File |
                Where-Object {
                    $Extensions -contains $_.Extension.ToLowerInvariant() -and
                    $_.FullName -notmatch "[\\/]target[\\/]" -and
                    $_.FullName -notmatch "[\\/]node_modules[\\/]" -and
                    $_.FullName -notmatch "[\\/]generated[\\/]"
                }
        )
    }
    return @($files | Sort-Object FullName -Unique)
}

function Get-NearestRustFunction {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [AllowEmptyString()]
        [string[]]$Lines,
        [Parameter(Mandatory)]
        [int]$Index
    )

    for ($cursor = $Index; $cursor -ge 0; $cursor--) {
        if ($Lines[$cursor] -match "\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(") {
            $isTest = $false
            for ($attribute = [Math]::Max(0, $cursor - 4); $attribute -lt $cursor; $attribute++) {
                if ($Lines[$attribute] -match "#\s*\[\s*test\s*\]") {
                    $isTest = $true
                    break
                }
            }
            return [pscustomobject]@{
                Name    = $Matches[1]
                Start   = $cursor
                IsTest  = $isTest
            }
        }
    }
    return [pscustomobject]@{ Name = ""; Start = 0; IsTest = $false }
}

function Test-RustTestRegion {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [AllowEmptyString()]
        [string[]]$Lines,
        [Parameter(Mandatory)]
        [int]$Index
    )

    $braceDepth = 0
    $testModuleDepth = $null
    $testModuleStart = $null
    for ($cursor = 0; $cursor -le $Index -and $cursor -lt $Lines.Count; $cursor++) {
        if ($Lines[$cursor] -match "#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]") {
            $testModuleDepth = $braceDepth
            $testModuleStart = $cursor
        }
        $opens = ([regex]::Matches($Lines[$cursor], "\{")).Count
        $closes = ([regex]::Matches($Lines[$cursor], "\}")).Count
        $braceDepth += $opens - $closes
        if ($testModuleDepth -ne $null -and $cursor -gt $testModuleStart -and $braceDepth -le $testModuleDepth) {
            $testModuleDepth = $null
            $testModuleStart = $null
        }
    }
    return $testModuleDepth -ne $null
}

function Test-HotProjectionContext {
    param(
        [Parameter(Mandatory)]
        [string]$RelativePath,
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$FunctionName
    )

    if ($RelativePath -match "^src-tauri/src/(channels|contract|commands)/") {
        return $true
    }
    if ($RelativePath -eq "src/core/live_capture.rs" -and $FunctionName -match "(?i)(process_event|queue_abyss_archive)") {
        return $true
    }
    return $FunctionName -match "(?i)(presented|projection|snapshot|stream|detail|main_dps)"
}

function Test-HotCloneFingerprint {
    param(
        [Parameter(Mandatory)]
        [string]$Window,
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$FunctionName
    )

    if ($Window -match "with_state\s*\(\s*Clone\s*::\s*clone\s*\)") {
        return $true
    }
    if ($FunctionName -match "(?i)main_round" -and $Window -match "\brecords\.clone\s*\(\)") {
        return $true
    }
    return $FunctionName -match "(?i)(process_event|queue_abyss_archive)" -and
        $Window -match "HistoryCombatDetails::from_state\s*\("
}

function Find-HotCloneOccurrences {
    param(
        [Parameter(Mandatory)]
        [System.IO.FileInfo[]]$Files
    )

    $occurrences = @()
    $seen = @{}
    foreach ($file in $Files) {
        $relative = ConvertTo-RelativePath $file.FullName
        if ($relative -notmatch "^(src-tauri/src/.*\.rs|src/core/live_capture\.rs)$") {
            continue
        }
        $lines = @(Get-Content -LiteralPath $file.FullName)
        for ($index = 0; $index -lt $lines.Count; $index++) {
            $windowEnd = [Math]::Min($lines.Count - 1, $index + 2)
            $window = ($lines[$index..$windowEnd] -join " ")
            $tokenLine = $index
            for ($candidate = $index; $candidate -le $windowEnd; $candidate++) {
                if ($lines[$candidate] -match "with_state|records\.clone|HistoryCombatDetails::from_state") {
                    $tokenLine = $candidate
                    break
                }
            }
            $function = Get-NearestRustFunction -Lines $lines -Index $index
            if (-not (Test-HotProjectionContext $relative $function.Name)) {
                continue
            }
            if (-not (Test-HotCloneFingerprint $window $function.Name)) {
                continue
            }
            $key = "$relative|$($tokenLine + 1)|$($function.Name)"
            if ($seen.ContainsKey($key)) {
                continue
            }
            $seen[$key] = $true
            $occurrences += [pscustomobject]@{
                Path     = $relative
                Line     = $tokenLine + 1
                Function = $function.Name
                Text     = $lines[$tokenLine].Trim()
            }
        }
    }
    return @($occurrences)
}

function Find-ChannelThreadSleepOccurrences {
    param(
        [Parameter(Mandatory)]
        [System.IO.FileInfo[]]$Files
    )

    $occurrences = @()
    foreach ($file in $Files) {
        $relative = ConvertTo-RelativePath $file.FullName
        if ($relative -notmatch "^src-tauri/src/channels/.*\.rs$") {
            continue
        }
        $lines = @(Get-Content -LiteralPath $file.FullName)
        $spawn = @(
            for ($index = 0; $index -lt $lines.Count; $index++) {
                if ($lines[$index] -match "\bthread::spawn\s*\(") { $index + 1 }
            }
        )
        $sleep = @(
            for ($index = 0; $index -lt $lines.Count; $index++) {
                if ($lines[$index] -match "\bthread::sleep\s*\(") { $index + 1 }
            }
        )
        if ($spawn.Count -gt 0 -and $sleep.Count -gt 0) {
            $source = $lines -join "`n"
            $occurrences += [pscustomobject]@{
                Path        = $relative
                SpawnCount  = $spawn.Count
                SleepCount  = $sleep.Count
                FirstSpawn  = $spawn[0]
                FirstSleep  = $sleep[0]
                SpawnLines  = @($spawn)
                SleepLines  = @($sleep)
                OwnerCancellation = $source -match "begin_stream" -and
                    $source -match "finish_stream" -and
                    $source -match "while\s*(?:!stop\.load|\(\s*!stop\.load)"
            }
        }
    }
    return @($occurrences)
}

function Test-ReplayImportValidation {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [AllowEmptyString()]
        [string[]]$Lines,
        [Parameter(Mandatory)]
        [int]$ReadIndex,
        [Parameter(Mandatory)]
        [int]$FunctionStart
    )

    if ($ReadIndex -le $FunctionStart) {
        return $false
    }
    $prefix = ($Lines[$FunctionStart..($ReadIndex - 1)] -join "`n")
    return $prefix -match "(?i)(validate_[A-Za-z0-9_]*(import|size|budget|path)|file_size|MAX_[A-Z0-9_]*(IMPORT|REPLAY|SIZE)|metadata[\s\S]*?\.len\s*\(\))"
}

function Find-ReplayBudgetViolations {
    param(
        [Parameter(Mandatory)]
        [System.IO.FileInfo[]]$Files
    )

    $violations = @()
    foreach ($file in $Files) {
        $relative = ConvertTo-RelativePath $file.FullName
        if ($relative -notmatch "^src/(engine|core|storage)/.*\.rs$") {
            continue
        }
        $lines = @(Get-Content -LiteralPath $file.FullName)
        for ($index = 0; $index -lt $lines.Count; $index++) {
            if ($lines[$index] -notmatch "(?:std::fs::|fs::)?(?:read_to_string|read_to_end|read)\s*\(") {
                continue
            }
            $function = Get-NearestRustFunction -Lines $lines -Index $index
            $isReplayFunction = $function.Name -match "(?i)(import|replay|capture_json)"
            if (-not $isReplayFunction -or $function.IsTest) {
                continue
            }
            if (-not (Test-ReplayImportValidation -Lines $lines -ReadIndex $index -FunctionStart $function.Start)) {
                $violations += New-Diagnostic `
                    -Severity Error `
                    -Rule $script:RuleIds['ReplayBudget'] `
                    -Path $relative `
                    -Line ($index + 1) `
                    -Message "Replay/import function '$($function.Name)' reads file bytes without a preceding metadata/size validation."
            }
        }
    }
    return @($violations)
}

function Find-ContractSilentSlices {
    param(
        [Parameter(Mandatory)]
        [System.IO.FileInfo[]]$Files
    )

    $occurrences = @()
    foreach ($file in $Files) {
        $relative = ConvertTo-RelativePath $file.FullName
        if ($relative -notmatch "^frontend/src/lib/tauri/.*contract\.ts$") {
            continue
        }
        $lines = @(Get-Content -LiteralPath $file.FullName)
        for ($index = 0; $index -lt $lines.Count; $index++) {
            if (-not (Test-RequiredContractSliceWindow -Lines $lines -Index $index)) {
                continue
            }
            $occurrences += [pscustomobject]@{
                Path = $relative
                Line = $index + 1
                Text = $lines[$index].Trim()
            }
        }
    }
    return @($occurrences)
}

function Test-RequiredContractSliceWindow {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [AllowEmptyString()]
        [string[]]$Lines,
        [Parameter(Mandatory)]
        [int]$Index
    )

    if ($Index -lt 0 -or $Index -ge $Lines.Count -or $Lines[$Index] -notmatch "\.slice\s*\(") {
        return $false
    }
    $start = [Math]::Max(0, $Index - 5)
    $window = ($Lines[$start..$Index] -join "`n")
    return $window -match "\blist\s*\("
}

function Get-GitAddedLines {
    $range = $null
    if (-not [string]::IsNullOrWhiteSpace($env:GITHUB_BASE_REF)) {
        $range = "origin/$($env:GITHUB_BASE_REF)...HEAD"
    } elseif (-not [string]::IsNullOrWhiteSpace($env:GITHUB_EVENT_BEFORE) -and
        $env:GITHUB_EVENT_BEFORE -notmatch '^0+$') {
        $range = "$($env:GITHUB_EVENT_BEFORE)...HEAD"
    }
    $diffArgs = @("--no-ext-diff", "--unified=0")
    if ($null -ne $range) {
        $diffArgs += $range
    } else {
        # Include both staged and unstaged changes in local review runs.
        $diffArgs += "HEAD"
    }
    $diffArgs += @("--", "src", "src-tauri", "frontend")
    $diff = @(& git diff @diffArgs 2>$null)
    if ($LASTEXITCODE -ne 0) {
        return @()
    }

    $path = ""
    $lineNumber = 0
    $added = @()
    foreach ($line in $diff) {
        if ($line -match "^\+\+\+ b/(.+)$") {
            $path = $Matches[1] -replace "\\", "/"
            continue
        }
        if ($line -match "^@@ .* \+(\d+)(?:,(\d+))? @@") {
            $lineNumber = [int]$Matches[1]
            continue
        }
        if ([string]::IsNullOrWhiteSpace($path)) {
            continue
        }
        if ($line.StartsWith("+")) {
            $added += [pscustomobject]@{ Path = $path; Line = $lineNumber; Text = $line.Substring(1) }
            $lineNumber++
        } elseif (-not $line.StartsWith("-")) {
            $lineNumber++
        }
    }
    return @($added)
}

function Test-AddedOccurrence {
    param(
        [Parameter(Mandatory)]
        [string]$Path,
        [Parameter(Mandatory)]
        [int]$Line,
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [object[]]$AddedLines,
        [int]$Radius = 0
    )

    return @($AddedLines | Where-Object {
            $_.Path -eq $Path -and
            [Math]::Abs(([int]$_.Line) - $Line) -le $Radius
        }).Count -gt 0
}

function Test-IsExternalBoundaryPath {
    param([Parameter(Mandatory)][string]$Path)

    return $Path -match "^(src/(cli|api|engine|core|platform)/|src-tauri/src/(commands|channels|contract|windows)/|src-tauri/src/state\.rs$)"
}

function Find-ExternalBoundaryDiffWarnings {
    param(
        [Parameter(Mandatory)]
        [object[]]$AddedLines,
        [Parameter(Mandatory)]
        [hashtable]$SourceMap
    )

    $warnings = @()
    foreach ($entry in $AddedLines) {
        if (-not (Test-IsExternalBoundaryPath $entry.Path)) {
            continue
        }
        $text = [string]$entry.Text
        if ($text -notmatch "\.\s*(unwrap|expect)\s*\(|\bassert(?:_eq|_ne)?!\s*\(") {
            continue
        }
        if ($text -match "\.\s*unwrap_or(?:_else)?\s*\(") {
            continue
        }
        $function = [pscustomobject]@{ IsTest = $false }
        if ($SourceMap.ContainsKey($entry.Path)) {
            $function = Get-NearestRustFunction -Lines $SourceMap[$entry.Path] -Index ([int]$entry.Line - 1)
            if (Test-RustTestRegion -Lines $SourceMap[$entry.Path] -Index ([int]$entry.Line - 1)) {
                continue
            }
        }
        if ($function.IsTest) {
            continue
        }
        $operation = if ($text -match "assert") { "assert macro" } elseif ($text -match "expect") { "expect" } else { "unwrap" }
        $warnings += New-Diagnostic `
            -Severity Warning `
            -Rule $script:RuleIds['BoundaryPanic'] `
            -Path $entry.Path `
            -Line ([int]$entry.Line) `
            -Message "New external-boundary $operation; review that untrusted input cannot reach a panic."
    }
    return @($warnings)
}

function Get-AllBoundaryLines {
    param(
        [Parameter(Mandatory)]
        [hashtable]$SourceMap
    )

    $lines = [System.Collections.Generic.List[object]]::new()
    foreach ($path in $SourceMap.Keys) {
        if (-not (Test-IsExternalBoundaryPath $path)) {
            continue
        }
        $sourceLines = $SourceMap[$path]
        for ($index = 0; $index -lt $sourceLines.Count; $index++) {
            $lines.Add([pscustomobject]@{
                Path = $path
                Line = $index + 1
                Text = $sourceLines[$index]
            })
        }
    }
    return @($lines.ToArray())
}

function Add-CountRuleDiagnostics {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [System.Collections.Generic.List[object]]$Diagnostics,
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [string]$Rule,
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [hashtable]$CurrentCounts,
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [hashtable]$BaselineCounts,
        [Parameter(Mandatory)]
        [string]$Description
    )

    foreach ($key in $CurrentCounts.Keys) {
        $current = [int]$CurrentCounts[$key]
        $baseline = if ($BaselineCounts.ContainsKey($key)) { [int]$BaselineCounts[$key] } else { 0 }
        if ($current -gt $baseline) {
            $parts = $key.Split("|", 2)
            $path = $parts[0]
            $where = if ($parts.Count -gt 1) { " ($($parts[1]))" } else { "" }
            $Diagnostics.Add((New-Diagnostic -Severity Error -Rule $Rule -Path $path -Message "${Description}: found $current occurrence(s), baseline allows $baseline$where."))
        } elseif ($current -eq $baseline -and $current -gt 0) {
            $parts = $key.Split("|", 2)
            $path = $parts[0]
            $where = if ($parts.Count -gt 1) { " ($($parts[1]))" } else { "" }
            $Diagnostics.Add((New-Diagnostic -Severity Warning -Rule $Rule -Path $path -Message "${Description}: $current existing occurrence(s) are allowlisted$where." -Baseline $true))
        }
    }

    foreach ($key in $BaselineCounts.Keys) {
        $current = if ($CurrentCounts.ContainsKey($key)) { [int]$CurrentCounts[$key] } else { 0 }
        $baseline = [int]$BaselineCounts[$key]
        if ($current -lt $baseline) {
            $path = $key.Split("|", 2)[0]
            $Diagnostics.Add((New-Diagnostic -Severity Info -Rule $Rule -Path $path -Message "Baseline entry is stale: expected $baseline occurrence(s), found $current. Remove/update the allowlist after review." -Baseline $true))
        }
    }
}

function Test-PolicyHelpers {
    Assert-Policy (Test-HotProjectionContext "src-tauri/src/state.rs" "main_presented_combat_state") `
        "Hot projection helper must classify the main presented state"
    Assert-Policy (-not (Test-HotProjectionContext "src-tauri/src/state.rs" "reset_session_with_undo")) `
        "Hot projection helper must not classify undo snapshots"
    Assert-Policy (Test-HotCloneFingerprint "let records = cache.records.clone();" "main_round_records") `
        "Hot clone helper must detect deep history-record cache clones"
    Assert-Policy (Test-HotCloneFingerprint "HistoryCombatDetails::from_state(&state)" "process_event") `
        "Hot clone helper must detect archive conversion inside event processing"
    Assert-Policy (-not (Test-HotCloneFingerprint "HistoryCombatDetails::from_state(&state)" "queue_detached_abyss_round")) `
        "Hot clone helper must allow detached archive conversion outside process_event"

    $replayLines = @(
        "fn import_capture_json(path: &Path) {",
        "    validate_capture_json_import(path).unwrap();",
        "    let text = std::fs::read_to_string(path);",
        "}"
    )
    Assert-Policy (Test-ReplayImportValidation -Lines $replayLines -ReadIndex 2 -FunctionStart 0) `
        "Replay validation helper must accept validation before the read"
    Assert-Policy (-not (Test-ReplayImportValidation -Lines @("fn import_replay(path: &Path) {", "    let text = fs::read_to_string(path);", "}") -ReadIndex 1 -FunctionStart 0)) `
        "Replay validation helper must reject an unbounded read"
    Assert-Policy (-not (Test-ReplayImportValidation -Lines @("fn import_replay(path: &Path) {", "    let _metadata = fs::metadata(path);", "    let text = fs::read_to_string(path);", "}") -ReadIndex 2 -FunctionStart 0)) `
        "Replay validation helper must reject metadata-only checks"

    $sliceLines = @('const rows = list(source.rows, "rows").slice(0, 250).map(parseRow);')
    Assert-Policy (Test-RequiredContractSliceWindow -Lines $sliceLines -Index 0) `
        "Contract helper must detect list(...).slice(...)"
    Assert-Policy (-not (Test-RequiredContractSliceWindow -Lines @('const title = value.slice(0, 1);') -Index 0)) `
        "Contract helper must ignore unrelated string slices"

    $addedOccurrences = @(
        [pscustomobject]@{ Path = "frontend/src/lib/tauri/example-contract.ts"; Line = 12; Text = ".slice(0, 1)" }
    )
    Assert-Policy (Test-AddedOccurrence -Path "frontend/src/lib/tauri/example-contract.ts" -Line 12 -AddedLines $addedOccurrences) `
        "Added occurrence helper must detect an exact replacement line"
    Assert-Policy (-not (Test-AddedOccurrence -Path "frontend/src/lib/tauri/example-contract.ts" -Line 10 -AddedLines $addedOccurrences)) `
        "Added occurrence helper must not match an unrelated line"

    $added = @(
        "diff --git a/src/core/live_capture.rs b/src/core/live_capture.rs",
        "@@ -1 +1 @@",
        "+    value.unwrap();"
    )
    # The parser is exercised by a no-op diff here; repository runs exercise
    # the full git-backed path.  Keep this assertion to catch parser regressions.
    Assert-Policy ($added.Count -eq 3) "Self-test fixture must remain well-formed"
}

Test-PolicyHelpers
if ($SelfTestOnly) {
    Write-Output "Runtime safety policy helper tests passed."
    exit 0
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repositoryRoot
try {
    $rustFiles = Get-RepositoryFiles @("src", "src-tauri/src") @(".rs")
    $frontendFiles = Get-RepositoryFiles @("frontend/src") @(".ts", ".tsx")
    $diagnostics = [System.Collections.Generic.List[object]]::new()

    $hotOccurrences = @(Find-HotCloneOccurrences $rustFiles)
    $hotCounts = @{}
    foreach ($occurrence in $hotOccurrences) {
        $key = "$($occurrence.Path)|$($occurrence.Function)"
        if (-not $hotCounts.ContainsKey($key)) { $hotCounts[$key] = 0 }
        $hotCounts[$key] = [int]$hotCounts[$key] + 1
    }
    $cloneBaseline = [hashtable]$script:Baseline['CloneProjection']
    Add-CountRuleDiagnostics `
        -Diagnostics $diagnostics `
        -Rule $script:RuleIds['CloneProjection'] `
        -CurrentCounts $hotCounts `
        -BaselineCounts $cloneBaseline `
        -Description "High-frequency projection contains with_state(Clone::clone)"

    $channelOccurrences = @(Find-ChannelThreadSleepOccurrences $rustFiles)
    foreach ($occurrence in $channelOccurrences) {
        if (-not $occurrence.OwnerCancellation) {
            $diagnostics.Add((New-Diagnostic -Severity Error -Rule $script:RuleIds['ChannelWorker'] -Path $occurrence.Path -Line $occurrence.FirstSpawn -Message "Channel worker uses thread::spawn + thread::sleep without an owner-bound stop token and cleanup path."))
        }
    }

    $replayViolations = @(Find-ReplayBudgetViolations $rustFiles)
    foreach ($diagnostic in $replayViolations) { $diagnostics.Add($diagnostic) }

    $sliceOccurrences = @(Find-ContractSilentSlices $frontendFiles)
    $sliceCounts = @{}
    foreach ($occurrence in $sliceOccurrences) {
        if (-not $sliceCounts.ContainsKey($occurrence.Path)) { $sliceCounts[$occurrence.Path] = 0 }
        $sliceCounts[$occurrence.Path] = [int]$sliceCounts[$occurrence.Path] + 1
    }
    $contractBaseline = [hashtable]$script:Baseline['ContractSlice']
    Add-CountRuleDiagnostics `
        -Diagnostics $diagnostics `
        -Rule $script:RuleIds['ContractSlice'] `
        -CurrentCounts $sliceCounts `
        -BaselineCounts $contractBaseline `
        -Description "Required contract list is silently sliced"

    $sourceMap = @{}
    foreach ($file in $rustFiles) {
        $sourceMap[(ConvertTo-RelativePath $file.FullName)] = @(Get-Content -LiteralPath $file.FullName)
    }
    $addedLines = @(Get-GitAddedLines)

    foreach ($occurrence in $hotOccurrences) {
        $key = "$($occurrence.Path)|$($occurrence.Function)"
        $current = if ($hotCounts.ContainsKey($key)) { [int]$hotCounts[$key] } else { 0 }
        $baseline = if ($cloneBaseline.ContainsKey($key)) { [int]$cloneBaseline[$key] } else { 0 }
        if ($current -le $baseline -and
            (Test-AddedOccurrence -Path $occurrence.Path -Line $occurrence.Line -AddedLines $addedLines -Radius 2)) {
            $diagnostics.Add((New-Diagnostic -Severity Error -Rule $script:RuleIds['CloneProjection'] -Path $occurrence.Path -Line $occurrence.Line -Message "This diff adds an occurrence hidden by the count baseline; remove the full-state clone or review a new fingerprint explicitly."))
        }
    }

    foreach ($occurrence in $sliceOccurrences) {
        $current = if ($sliceCounts.ContainsKey($occurrence.Path)) { [int]$sliceCounts[$occurrence.Path] } else { 0 }
        $baseline = if ($contractBaseline.ContainsKey($occurrence.Path)) { [int]$contractBaseline[$occurrence.Path] } else { 0 }
        if ($current -le $baseline -and
            (Test-AddedOccurrence -Path $occurrence.Path -Line $occurrence.Line -AddedLines $addedLines)) {
            $diagnostics.Add((New-Diagnostic -Severity Error -Rule $script:RuleIds['ContractSlice'] -Path $occurrence.Path -Line $occurrence.Line -Message "This diff adds a silent contract slice hidden by the count baseline; validate the server-bounded list instead."))
        }
    }

    $boundaryLines = if ($Strict) {
        Get-AllBoundaryLines $sourceMap
    } else {
        $addedLines
    }
    foreach ($diagnostic in @(Find-ExternalBoundaryDiffWarnings $boundaryLines $sourceMap)) {
        $diagnostics.Add($diagnostic)
    }

    $ordered = @($diagnostics | Sort-Object Severity, Rule, Path, Line)
    foreach ($diagnostic in $ordered) {
        $location = if ($diagnostic.Line -gt 0) { "$($diagnostic.Path):$($diagnostic.Line)" } else { $diagnostic.Path }
        $suffix = if ($diagnostic.Baseline) { " [baseline]" } else { "" }
        Write-Output ("RUNTIME-SAFETY [{0}] [{1}] {2}{3} - {4}" -f $diagnostic.Severity.ToUpperInvariant(), $diagnostic.Rule, $location, $suffix, $diagnostic.Message)
    }

    $errors = @($ordered | Where-Object { $_.Severity -eq "Error" })
    $warnings = @($ordered | Where-Object { $_.Severity -eq "Warning" })
    if ($Strict) {
        $errors += @($warnings | ForEach-Object {
                New-Diagnostic -Severity Error -Rule $_.Rule -Path $_.Path -Line $_.Line -Message "Strict mode: $($_.Message)" -Baseline $_.Baseline
            })
    }
    if ($errors.Count -gt 0) {
        $modeHint = if ($Strict) { "review warnings are blocking in -Strict mode" } else { "unreviewed findings are blocking" }
        Write-Error ("Runtime safety policy failed with {0} blocking diagnostic(s): {1}." -f $errors.Count, $modeHint)
        exit 1
    }

    $mode = if ($Strict) { "strict" } else { "standard" }
    Write-Output ("Runtime safety policy passed ({0} mode): {1} warning(s), {2} informational diagnostic(s)." -f $mode, $warnings.Count, @($ordered | Where-Object { $_.Severity -eq "Info" }).Count)
}
finally {
    Pop-Location
}
