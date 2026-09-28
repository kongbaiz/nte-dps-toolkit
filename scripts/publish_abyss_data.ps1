<#
Deployment configuration is supplied through parameters or process environment:
  NTE_DEPLOY_SSH_HOST: SSH config alias or login target (required).
  NTE_ABYSS_REMOTE_ROOT: dedicated absolute server directory (required).
  NTE_DEPLOY_IDENTITY_FILE: optional identity; omit to use SSH config or agent.
Keep actual values in private operator configuration, never in this repository.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$SourceDirectory,
    [string]$SshHost = $env:NTE_DEPLOY_SSH_HOST,
    [string]$IdentityFile = $env:NTE_DEPLOY_IDENTITY_FILE,
    [string]$RemoteRoot = $env:NTE_ABYSS_REMOTE_ROOT,
    [string]$OutputDirectory = "target/abyss-data-deploy"
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

# Keep deployment identity and server layout in the operator's environment or SSH config.
if ([string]::IsNullOrWhiteSpace($SshHost) -or
    $SshHost -notmatch '^(?:[A-Za-z0-9_][A-Za-z0-9_.-]*@)?[A-Za-z0-9][A-Za-z0-9.-]*$') {
    throw "Set SshHost or NTE_DEPLOY_SSH_HOST to an SSH host alias or login target."
}
$RemoteRoot = $RemoteRoot.TrimEnd('/')
if ($RemoteRoot -notmatch '^/(?:[A-Za-z0-9_.-]+/)+[A-Za-z0-9_.-]+$' -or
    @($RemoteRoot.Split('/') | Where-Object { $_ -in @('.', '..') }).Count -ne 0) {
    throw "Set RemoteRoot or NTE_ABYSS_REMOTE_ROOT to a dedicated absolute deployment directory."
}
$sshOptions = @('-o', 'BatchMode=yes')
if (-not [string]::IsNullOrWhiteSpace($IdentityFile)) {
    if (-not (Test-Path -LiteralPath $IdentityFile -PathType Leaf)) {
        throw "The configured deployment identity file is unavailable."
    }
    $sshOptions += @('-i', $IdentityFile)
}

function Invoke-DeploymentCommand {
    param([string]$Command, [string[]]$Arguments, [string]$Operation)

    # Native diagnostics can contain private login targets, key paths and remote paths.
    $PSNativeCommandUseErrorActionPreference = $false
    try {
        $null = & $Command @Arguments 2>&1
        $commandExitCode = $LASTEXITCODE
    } catch {
        throw "Deployment command failed ($Operation). Check the private deployment configuration."
    }
    if ($commandExitCode -ne 0) {
        throw "Deployment command failed ($Operation, exit $commandExitCode). Check the private deployment configuration."
    }
}

& python scripts/build_abyss_data_package.py --source $SourceDirectory --output $OutputDirectory
$manifest = Get-Content -LiteralPath "$OutputDirectory/manifest.json" -Raw | ConvertFrom-Json
$sha256 = [string]$manifest.artifact.sha256
if ($sha256 -notmatch '^[0-9a-f]{64}$') {
    throw "Generated abyss manifest contains an invalid SHA-256"
}
$artifact = "abyss-data-$sha256.zip"
$remoteStaging = "$RemoteRoot/.staging-$sha256"

Invoke-DeploymentCommand -Command ssh -Operation 'prepare' -Arguments ($sshOptions + @(
    $SshHost, "set -eu; rm -rf '$remoteStaging'; install -d -m 0755 '$remoteStaging'"
))
Invoke-DeploymentCommand -Command scp -Operation 'upload' -Arguments ($sshOptions + @(
    "$OutputDirectory/manifest.json", "$OutputDirectory/$artifact", "${SshHost}:$remoteStaging/"
))
$publishCommand = @"
set -eu
cd '$remoteStaging'
printf '%s  %s\n' '$sha256' '$artifact' | sha256sum -c -
test "`$(stat -c %s '$artifact')" = '$($manifest.artifact.size)'
chmod 0644 manifest.json '$artifact'
install -d -m 0755 '$RemoteRoot/releases'
rm -rf '$RemoteRoot/releases/$sha256'
mv '$remoteStaging' '$RemoteRoot/releases/$sha256'
ln -sfn 'releases/$sha256' '$RemoteRoot/current.next'
mv -Tf '$RemoteRoot/current.next' '$RemoteRoot/current'
find '$RemoteRoot/releases' -mindepth 1 -maxdepth 1 -type d ! -name '$sha256' -exec rm -rf -- {} +
"@
Invoke-DeploymentCommand -Command ssh -Operation 'publish' -Arguments ($sshOptions + @($SshHost, $publishCommand))

Write-Output "Published $artifact ($($manifest.artifact.size) compressed bytes)"
