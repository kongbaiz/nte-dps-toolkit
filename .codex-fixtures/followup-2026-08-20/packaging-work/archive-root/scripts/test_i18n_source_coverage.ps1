[CmdletBinding()]
param(
    [switch]$SelfTestOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptRoot

function Get-I18nLiteralKeys {
    param([Parameter(Mandatory)][string]$Text)

    $patterns = @(
        [regex]::new('\b(?:t|tf)\(\s*["'']([^"'']+)["'']', [System.Text.RegularExpressions.RegexOptions]::CultureInvariant),
        [regex]::new('message_key\s*:\s*["'']([^"'']+)["'']', [System.Text.RegularExpressions.RegexOptions]::CultureInvariant),
        [regex]::new('CommandError::[A-Za-z0-9_]+\(\s*["''][^"'']+["'']\s*,\s*["'']([^"'']+)["'']', [System.Text.RegularExpressions.RegexOptions]::CultureInvariant)
    )

    $keys = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    foreach ($pattern in $patterns) {
        foreach ($match in $pattern.Matches($Text)) {
            [void]$keys.Add($match.Groups[1].Value)
        }
    }
    return $keys
}

function Assert-SetEqual {
    param(
        [Parameter(Mandatory)][System.Collections.Generic.HashSet[string]]$Expected,
        [Parameter(Mandatory)][System.Collections.Generic.HashSet[string]]$Actual,
        [Parameter(Mandatory)][string]$Message
    )

    if (-not $Expected.SetEquals($Actual)) {
        $missing = @($Expected | Where-Object { -not $Actual.Contains($_) } | Sort-Object)
        $extra = @($Actual | Where-Object { -not $Expected.Contains($_) } | Sort-Object)
        throw "$Message missing=[$($missing -join '; ')] extra=[$($extra -join '; ')]"
    }
}

function Assert-NoDuplicateLocaleKeys {
    param(
        [Parameter(Mandatory)][string]$Json,
        [Parameter(Mandatory)][string]$Label
    )

    # Locale resources are deliberately flat string maps. Inspect the raw JSON
    # before ConvertFrom-Json, which otherwise keeps only the last duplicate.
    $seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    $duplicates = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    foreach ($match in [regex]::Matches($Json, '(?m)^\s*"(?<key>(?:\\.|[^"\\])*)"\s*:')) {
        $encodedKey = $match.Groups['key'].Value
        if (-not $seen.Add($encodedKey)) {
            [void]$duplicates.Add($encodedKey)
        }
    }
    if ($duplicates.Count -gt 0) {
        throw "$Label contains duplicate JSON keys: $(@($duplicates | Sort-Object) -join '; ')"
    }
}

# Keep the source recognizers deterministic so a future refactor cannot turn
# this gate into a successful no-op.
$selfTest = @'
t("Frontend literal")
message_key: "Rust field"
CommandError::main_dps(
    "stable_code",
    "Rust constructor argument",
)
'@
$expectedSelfTest = [System.Collections.Generic.HashSet[string]]::new(
    [string[]]@("Frontend literal", "Rust field", "Rust constructor argument"),
    [System.StringComparer]::Ordinal
)
$actualSelfTest = Get-I18nLiteralKeys -Text $selfTest
Assert-SetEqual -Expected $expectedSelfTest -Actual $actualSelfTest -Message "i18n source recognizer self-test failed"
$duplicateSelfTestRejected = $false
try {
    Assert-NoDuplicateLocaleKeys -Json "{`n  `"Same`": `"one`",`n  `"Same`": `"two`"`n}" -Label "fixture"
}
catch {
    $duplicateSelfTestRejected = $_.Exception.Message -match 'duplicate JSON keys'
}
if (-not $duplicateSelfTestRejected) {
    throw "i18n duplicate-key self-test accepted an ambiguous locale map"
}

if ($SelfTestOnly) {
    Write-Output "I18N source coverage self-test passed: literalPatterns=3 duplicateKeysRejected=true."
    exit 0
}

$localePaths = @{
    "zh-CN" = Join-Path $repoRoot "res/languages/zh-CN.json"
    "ja" = Join-Path $repoRoot "res/languages/ja.json"
}
$localeMaps = @{}
$localeKeySets = @{}
foreach ($locale in $localePaths.Keys) {
    $json = Get-Content -LiteralPath $localePaths[$locale] -Raw -Encoding UTF8
    Assert-NoDuplicateLocaleKeys -Json $json -Label $localePaths[$locale]
    $map = $json | ConvertFrom-Json -AsHashtable
    $localeMaps[$locale] = $map
    $localeKeySets[$locale] = [System.Collections.Generic.HashSet[string]]::new(
        [string[]]@($map.Keys),
        [System.StringComparer]::Ordinal
    )
}
Assert-SetEqual `
    -Expected $localeKeySets["zh-CN"] `
    -Actual $localeKeySets["ja"] `
    -Message "locale dictionaries have different key sets"

$sourceKeys = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
$sourceLocations = @{}
$sourceRoots = @(
    (Join-Path $repoRoot "frontend/src")
    (Join-Path $repoRoot "src")
    (Join-Path $repoRoot "src-tauri/src")
)
foreach ($sourceRoot in $sourceRoots) {
    $files = Get-ChildItem -LiteralPath $sourceRoot -Recurse -File |
        Where-Object {
            $_.Extension -in @(".rs", ".ts", ".tsx") -and
            $_.Name -notmatch '\.(?:test|spec)\.'
        }
    foreach ($file in $files) {
        $text = [System.IO.File]::ReadAllText($file.FullName)
        if ($file.Extension -eq ".rs") {
            $testMarker = [regex]::Match(
                $text,
                '(?m)^\s*#\s*\[\s*cfg\s*\(\s*(?:all\s*\(\s*)?test'
            )
            if ($testMarker.Success) {
                $text = $text.Substring(0, $testMarker.Index)
            }
        }
        foreach ($key in (Get-I18nLiteralKeys -Text $text)) {
            [void]$sourceKeys.Add($key)
            if (-not $sourceLocations.ContainsKey($key)) {
                $sourceLocations[$key] = $file.FullName.Substring($repoRoot.Length + 1)
            }
        }
    }
}

$missing = @($sourceKeys | Where-Object { -not $localeKeySets["zh-CN"].Contains($_) } | Sort-Object)
if ($missing.Count -gt 0) {
    $details = $missing | ForEach-Object { "$_ [$($sourceLocations[$_])]" }
    throw "source i18n keys are missing from both locale dictionaries: $($details -join '; ')"
}

Write-Output (
    "I18N source coverage passed: sourceKeys={0} localeKeys={1} parity=true duplicateKeysRejected=true." -f `
        $sourceKeys.Count,
        $localeKeySets["zh-CN"].Count
)
