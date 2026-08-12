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

$developerShell = Join-Path $installationPath "Common7\Tools\VsDevCmd.bat"
New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null

$command = @(
    'call', ('"{0}"' -f $developerShell), '-arch=x64', '-host_arch=x64', '>', 'nul', '&&',
    'cl.exe', '/nologo', '/std:c++20', '/W4', '/WX', '/EHsc', '/c',
    ('"{0}"' -f $source), ('/Fo"{0}"' -f $objectFile)
) -join ' '

& $env:ComSpec /d /s /c $command
if ($LASTEXITCODE -ne 0) {
    throw "Native signature policy tests failed with exit code $LASTEXITCODE."
}

Write-Output "signature_policy_tests: PASS"
