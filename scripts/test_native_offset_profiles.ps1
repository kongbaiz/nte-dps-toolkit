param(
    [ValidateSet("baseline","modified")]
    [string]$Mode = "modified"
)
$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$testSource = Join-Path $repoRoot "native\nte-mods-plugin\tests\offset_profile_tests.cpp"
$memorySource = Join-Path $repoRoot "native\nte-mods-plugin\src\memory_access.cpp"
if ($Mode -eq "baseline") {
    $resolverSource = Join-Path $repoRoot "artifacts\mod_remove_checksum_gate_20260809\offset_resolver.baseline.cpp"
    $expected = "production_known=true.*test_known=true.*small_update_known=false"
} else {
    $resolverSource = Join-Path $repoRoot "native\nte-mods-plugin\src\offset_resolver.cpp"
    $expected = "production_known=true.*test_known=true.*small_update_known=true"
}
$outputDirectory = Join-Path $env:TEMP ("nte-mods-plugin-offset-profile-tests-" + $Mode)
$exe = Join-Path $outputDirectory "offset_profile_tests.exe"
New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
$vswhere = 'C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe'
$installationPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
$developerShell = Join-Path $installationPath "Common7\Tools\VsDevCmd.bat"
$sources = @($testSource, $resolverSource, $memorySource)
foreach ($source in $sources) {
    $object = Join-Path $outputDirectory (([System.IO.Path]::GetFileNameWithoutExtension($source)) + ".obj")
    $compileCommand = @(
        'call', ('"{0}"' -f $developerShell), '-arch=x64', '-host_arch=x64', '>', 'nul', '&&',
        'cl.exe', '/nologo', '/std:c++20', '/W4', '/WX', '/EHsc', '/c',
        ('/I"{0}"' -f (Join-Path $repoRoot "native\nte-mods-plugin\src")),
        ('"{0}"' -f $source), ('/Fo"{0}"' -f $object)
    ) -join ' '
    & $env:ComSpec /d /s /c $compileCommand
    if ($LASTEXITCODE -ne 0) { throw ("compile failed for {0}: {1}" -f $source, $LASTEXITCODE) }
}
$resolverObject = Join-Path $outputDirectory (([System.IO.Path]::GetFileNameWithoutExtension($resolverSource)) + ".obj")
$linkCommand = @(
    'call', ('"{0}"' -f $developerShell), '-arch=x64', '-host_arch=x64', '>', 'nul', '&&',
    'link.exe', '/nologo', '/subsystem:console', ('/out:{0}' -f $exe),
    ('"{0}\offset_profile_tests.obj"' -f $outputDirectory),
    ('"{0}"' -f $resolverObject),
    ('"{0}\memory_access.obj"' -f $outputDirectory),
    'kernel32.lib'
) -join ' '
& $env:ComSpec /d /s /c $linkCommand
if ($LASTEXITCODE -ne 0) { throw "link failed: $LASTEXITCODE" }
$output = & $exe
$exitCode = $LASTEXITCODE
Write-Output $output
$outputText = $output -join "`n"
if ($exitCode -ne 0 -or $outputText -notmatch $expected) { exit 1 }
Write-Output ("offset_profile_tests mode={0}: PASS" -f $Mode)
exit 0
