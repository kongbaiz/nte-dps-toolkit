[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$SourceDirectory,
    [string]$SshHost = "deploy@example.invalid",
    [string]$IdentityFile = "$env:NTE_DEPLOY_IDENTITY_FILE",
    [string]$OutputDirectory = "target/abyss-data-deploy"
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

& python scripts/build_abyss_data_package.py --source $SourceDirectory --output $OutputDirectory
$manifest = Get-Content -LiteralPath "$OutputDirectory/manifest.json" -Raw | ConvertFrom-Json
$sha256 = [string]$manifest.artifact.sha256
if ($sha256 -notmatch '^[0-9a-f]{64}$') {
    throw "Generated abyss manifest contains an invalid SHA-256"
}
$artifact = "abyss-data-$sha256.zip"
$remoteStaging = "/srv/example-deployment/.staging-$sha256"

ssh -i $IdentityFile -o BatchMode=yes $SshHost `
    "set -eu; rm -rf '$remoteStaging'; install -d -m 0755 '$remoteStaging'"
scp -i $IdentityFile -o BatchMode=yes `
    "$OutputDirectory/manifest.json" `
    "$OutputDirectory/$artifact" `
    "${SshHost}:$remoteStaging/"
ssh -i $IdentityFile -o BatchMode=yes $SshHost @"
set -eu
cd '$remoteStaging'
printf '%s  %s\n' '$sha256' '$artifact' | sha256sum -c -
test "`$(stat -c %s '$artifact')" = '$($manifest.artifact.size)'
chmod 0644 manifest.json '$artifact'
install -d -m 0755 /srv/example-deployment/releases
rm -rf '/srv/example-deployment/releases/$sha256'
mv '$remoteStaging' '/srv/example-deployment/releases/$sha256'
ln -sfn 'releases/$sha256' /srv/example-deployment/current.next
mv -Tf /srv/example-deployment/current.next /srv/example-deployment/current
find /srv/example-deployment/releases -mindepth 1 -maxdepth 1 -type d ! -name '$sha256' -exec rm -rf -- {} +
"@

Write-Output "Published $artifact ($($manifest.artifact.size) compressed bytes)"
