param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('nte-publish-privacy-' + [Guid]::NewGuid().ToString('N'))
$output = Join-Path $testRoot 'output'
$identity = Join-Path $testRoot 'private-identity-marker'
$publisher = Join-Path $PSScriptRoot 'publish_abyss_data.ps1'
$privacyFixture = @{
    BuildCalls = 0
    RemoteCalls = [Collections.Generic.List[object]]::new()
    RemoteExitCode = 0
    ArtifactHash = 'a' * 64
}

# Replace every external command used by deployment. No network or real keys are used.
function python {
    $privacyFixture.BuildCalls++
    New-Item -ItemType Directory -Path $output -Force | Out-Null
    @{ artifact = @{ sha256 = $privacyFixture.ArtifactHash; size = 4 } } |
        ConvertTo-Json -Compress | Set-Content (Join-Path $output 'manifest.json')
    $global:LASTEXITCODE = 0
}
function ssh {
    $privacyFixture.RemoteCalls.Add(@{ command = 'ssh'; arguments = @($args) })
    Write-Output "sensitive-native-output $identity private-login@example.invalid"
    $global:LASTEXITCODE = $privacyFixture.RemoteExitCode
}
function scp {
    $privacyFixture.RemoteCalls.Add(@{ command = 'scp'; arguments = @($args) })
    Write-Output "sensitive-native-output $identity private-login@example.invalid"
    $global:LASTEXITCODE = $privacyFixture.RemoteExitCode
}
function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
function Invoke-TestPublish([hashtable]$Overrides = @{}) {
    $arguments = @{
        SourceDirectory = 'fixture-source'
        OutputDirectory = $output
        SshHost = 'fixture-alias'
        RemoteRoot = '/srv/fixture-release'
        IdentityFile = ''
    }
    foreach ($key in $Overrides.Keys) { $arguments[$key] = $Overrides[$key] }
    & $publisher @arguments
}

try {
    New-Item -ItemType Directory -Path $testRoot | Out-Null
    Set-Content -LiteralPath $identity -Value 'fixture, not a private key'
    foreach ($invalid in @(
        @{ SshHost = '' },
        @{ SshHost = '-oProxyCommand=invalid' },
        @{ SshHost = "fixture'; command" },
        @{ RemoteRoot = '' },
        @{ RemoteRoot = '/' },
        @{ RemoteRoot = '/srv' },
        @{ RemoteRoot = '/srv/../private' },
        @{ RemoteRoot = "/srv/fixture'; command" },
        @{ IdentityFile = (Join-Path $testRoot 'missing-private-marker') }
    )) {
        $failed = $false
        try { Invoke-TestPublish $invalid | Out-Null } catch {
            $failed = $true
            Assert-True (-not $_.Exception.Message.Contains($testRoot)) 'Validation exposed a private path.'
        }
        Assert-True $failed 'Invalid deployment configuration was accepted.'
        Assert-True ($privacyFixture.BuildCalls -eq 0 -and $privacyFixture.RemoteCalls.Count -eq 0) 'Invalid configuration caused a side effect.'
    }

    $result = (Invoke-TestPublish | Out-String)
    Assert-True ($privacyFixture.RemoteCalls.Count -eq 3) 'Expected prepare, upload and publish.'
    Assert-True (-not $result.Contains('sensitive-native-output')) 'Native output was exposed.'
    Assert-True ($privacyFixture.RemoteCalls[0].arguments -notcontains '-i') 'Default SSH configuration was not used.'
    $publish = $privacyFixture.RemoteCalls[2].arguments[-1]
    Assert-True ($publish.Contains("'/srv/fixture-release/current'")) 'Configured deployment directory was not used.'
    Assert-True ($result.Contains('Published abyss-data-')) 'Publication result is missing.'

    $privacyFixture.RemoteCalls.Clear()
    Invoke-TestPublish @{ IdentityFile = $identity } | Out-Null
    Assert-True ($privacyFixture.RemoteCalls[0].arguments -contains $identity) 'Explicit identity was not passed to SSH.'

    $privacyFixture.RemoteCalls.Clear()
    $privacyFixture.RemoteExitCode = 23
    $failed = $false
    try { Invoke-TestPublish | Out-Null } catch {
        $failed = $true
        $message = $_.Exception.Message
        Assert-True ($message.Contains('prepare, exit 23')) 'Failure lost operation and exit code.'
        Assert-True (-not $message.Contains('sensitive-native-output') -and -not $message.Contains($testRoot)) 'Failure exposed native diagnostics.'
    }
    Assert-True $failed 'Remote failure was accepted.'
    Assert-True ($privacyFixture.RemoteCalls.Count -eq 1) 'Deployment continued after prepare failed.'
    Write-Output 'PUBLISH-PRIVACY: PASS configuration validation, SSH configuration, redacted output, failure stops deployment'
} finally {
    $resolved = [IO.Path]::GetFullPath($testRoot)
    $tempParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([IO.Path]::DirectorySeparatorChar)
    if ((Split-Path -Parent $resolved) -eq $tempParent -and
        (Split-Path -Leaf $resolved).StartsWith('nte-publish-privacy-') -and
        (Test-Path -LiteralPath $resolved)) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
