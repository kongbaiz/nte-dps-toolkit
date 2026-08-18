$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$outputDirectory = Join-Path $env:TEMP "nte-mods-plugin-find-offsets-tests"
$exe = Join-Path $outputDirectory "find_offsets_scanner_tests.exe"
New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null

$knownProfiles = & rg -n "KnownOffsetProfile|KNOWN_OFFSET_PROFILES|IsKnownImageProfile|ResolutionSource::KnownProfile" `
    (Join-Path $repoRoot "native\nte-mods-plugin\src")
if ($LASTEXITCODE -eq 0 -or $knownProfiles) {
    Write-Error "Known offset profile state is still present:`n$knownProfiles"
}
if ($LASTEXITCODE -ne 1) { exit 1 }

$vswhere = 'C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe'
$installationPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
$developerShell = Join-Path $installationPath "Common7\Tools\VsDevCmd.bat"
$sources = @(
    (Join-Path $repoRoot "native\nte-mods-plugin\tests\find_offsets_scanner_tests.cpp"),
    (Join-Path $repoRoot "native\nte-mods-plugin\src\find_offsets_scanner.cpp"),
    (Join-Path $repoRoot "native\nte-mods-plugin\src\offset_resolver.cpp"),
    (Join-Path $repoRoot "native\nte-mods-plugin\src\memory_access.cpp")
)
$objects = @()
foreach ($source in $sources) {
    $object = Join-Path $outputDirectory (([System.IO.Path]::GetFileNameWithoutExtension($source)) + ".obj")
    $objects += $object
    $compileCommand = @(
        'call', ('"{0}"' -f $developerShell), '-arch=x64', '-host_arch=x64', '>', 'nul', '&&',
        'cl.exe', '/nologo', '/std:c++20', '/utf-8', '/W4', '/WX', '/EHsc', '/DNOMINMAX', '/c',
        ('/I"{0}"' -f (Join-Path $repoRoot "native\nte-mods-plugin\src")),
        ('"{0}"' -f $source), ('/Fo"{0}"' -f $object)
    ) -join ' '
    & $env:ComSpec /d /s /c $compileCommand
    if ($LASTEXITCODE -ne 0) { throw "compile failed for ${source}: $LASTEXITCODE" }
}
$linkCommand = @(
    'call', ('"{0}"' -f $developerShell), '-arch=x64', '-host_arch=x64', '>', 'nul', '&&',
    'link.exe', '/nologo', '/subsystem:console', ('/out:"{0}"' -f $exe)
) + ($objects | ForEach-Object { '"{0}"' -f $_ }) + @('kernel32.lib')
& $env:ComSpec /d /s /c ($linkCommand -join ' ')
if ($LASTEXITCODE -ne 0) { throw "link failed: $LASTEXITCODE" }
$output = & $exe
$exitCode = $LASTEXITCODE
Write-Output $output
if ($exitCode -ne 0 -or ($output -join "`n") -notmatch 'known_profiles=absent') { exit 1 }
Write-Output "find_offsets_scanner_tests: PASS"
exit 0
