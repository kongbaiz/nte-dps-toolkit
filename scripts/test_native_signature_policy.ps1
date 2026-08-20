$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$source = Join-Path $repoRoot "native\nte-mods-plugin\tests\signature_policy_tests.cpp"
$outputDirectory = Join-Path $env:TEMP "nte-mods-plugin-signature-tests"
$objectFile = Join-Path $outputDirectory "signature_policy_tests.obj"

$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path -LiteralPath $vswhere)) {
    throw "vswhere.exe was not found."
}

$installationPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $installationPath) {
    throw "Visual C++ x64 build tools were not found."
}

$toolset = Get-ChildItem (Join-Path $installationPath 'VC\Tools\MSVC') -Directory |
    Sort-Object Name -Descending |
    Select-Object -First 1
$sdkRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10'
$sdk = Get-ChildItem (Join-Path $sdkRoot 'Include') -Directory |
    Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'um\Windows.h') } |
    Sort-Object Name -Descending |
    Select-Object -First 1
if (-not $toolset -or -not $sdk) {
    throw 'The Visual C++ or Windows SDK x64 toolchain was not found.'
}
$compiler = Join-Path $toolset.FullName 'bin\Hostx64\x64\cl.exe'
New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null

$arguments = @(
    '/nologo', '/std:c++20', '/utf-8', '/W4', '/WX', '/EHsc', '/GS', '/sdl', '/c',
    ('/I' + (Join-Path $toolset.FullName 'include')),
    ('/I' + (Join-Path $sdk.FullName 'ucrt')),
    ('/I' + (Join-Path $sdk.FullName 'shared')),
    ('/I' + (Join-Path $sdk.FullName 'um')),
    ('/I' + (Join-Path $sdk.FullName 'winrt')),
    $source,
    ('/Fo:' + $objectFile)
)

Push-Location $outputDirectory
try {
    & $compiler @arguments
}
finally {
    Pop-Location
}
if ($LASTEXITCODE -ne 0) {
    throw "Native signature policy tests failed with exit code $LASTEXITCODE."
}

Write-Output "signature_policy_tests: PASS"
