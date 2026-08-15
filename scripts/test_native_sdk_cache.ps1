$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$outputDirectory = Join-Path $env:TEMP "nte-mods-plugin-sdk-cache-tests"
$exe = Join-Path $outputDirectory "sdk_cache_tests.exe"
New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
$vswhere = 'C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe'
$installationPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
$developerShell = Join-Path $installationPath "Common7\Tools\VsDevCmd.bat"
$sources = @(
    (Join-Path $repoRoot "native\nte-mods-plugin\tests\sdk_cache_tests.cpp"),
    (Join-Path $repoRoot "native\nte-mods-plugin\src\sdk_cache.cpp")
)
$objects = @()
foreach ($source in $sources) {
    $object = Join-Path $outputDirectory (([System.IO.Path]::GetFileNameWithoutExtension($source)) + ".obj")
    $objects += $object
    $compileCommand = @(
        'call', ('"{0}"' -f $developerShell), '-arch=x64', '-host_arch=x64', '>', 'nul', '&&',
        'cl.exe', '/nologo', '/std:c++20', '/W4', '/WX', '/EHsc', '/c',
        ('/I"{0}"' -f (Join-Path $repoRoot "native\nte-mods-plugin\src")),
        ('"{0}"' -f $source), ('/Fo"{0}"' -f $object)
    ) -join ' '
    & $env:ComSpec /d /s /c $compileCommand
    if ($LASTEXITCODE -ne 0) { throw "compile failed for ${source}: $LASTEXITCODE" }
}
$linkCommand = @(
    'call', ('"{0}"' -f $developerShell), '-arch=x64', '-host_arch=x64', '>', 'nul', '&&',
    'link.exe', '/nologo', '/subsystem:console', ('/out:"{0}"' -f $exe)
) + ($objects | ForEach-Object { '"{0}"' -f $_ }) + @('bcrypt.lib', 'kernel32.lib')
& $env:ComSpec /d /s /c ($linkCommand -join ' ')
if ($LASTEXITCODE -ne 0) { throw "link failed: $LASTEXITCODE" }
$output = & $exe
$exitCode = $LASTEXITCODE
Write-Output $output
if ($exitCode -ne 0 -or ($output -join "`n") -notmatch 'failed_publish_preserved=true.*sdk_only=true') { exit 1 }
Write-Output "sdk_cache_tests: PASS"
exit 0
