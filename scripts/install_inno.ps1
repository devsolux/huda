# CI build-tool setup only. Never invoked by Huda or dev.sh.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$Version = '7.1.0'
$Tag = 'is-7_1_0'
$Name = "innosetup-$Version-x64.exe"
$Headers = @{ 'User-Agent' = 'Huda-native-build'; 'Accept' = 'application/vnd.github+json' }
# Authenticated metadata reads avoid the shared anonymous API rate limit on hosted runners.
# The token leaves the environment before the downloaded installer runs.
if ($env:GITHUB_TOKEN) {
    $Headers['Authorization'] = "Bearer $env:GITHUB_TOKEN"
    Remove-Item Env:GITHUB_TOKEN
}
$Release = Invoke-RestMethod -Uri "https://api.github.com/repos/jrsoftware/issrc/releases/tags/$Tag" -Headers $Headers
$Assets = @($Release.assets | Where-Object { $_.name -eq $Name })
if ($Assets.Count -ne 1 -or $Assets[0].digest -notmatch '^sha256:[0-9a-f]{64}$') {
    throw 'The immutable installer release does not provide one SHA-256-identified asset.'
}
$Directory = Join-Path $env:RUNNER_TEMP 'huda-inno-7.1.0'
New-Item -ItemType Directory -Path $Directory | Out-Null
$Installer = Join-Path $Directory $Name
$Url = "https://github.com/jrsoftware/issrc/releases/download/$Tag/$Name"
Invoke-WebRequest -Uri $Url -OutFile $Installer
$Actual = (Get-FileHash -Path $Installer -Algorithm SHA256).Hash.ToLowerInvariant()
if ("sha256:$Actual" -ne $Assets[0].digest) { throw 'Inno Setup SHA-256 mismatch.' }
$Signature = Get-AuthenticodeSignature -FilePath $Installer
if ($Signature.Status -ne 'Valid' -or $null -eq $Signature.SignerCertificate) { throw 'Inno Setup signature is invalid.' }
$Publisher = $Signature.SignerCertificate.GetNameInfo([System.Security.Cryptography.X509Certificates.X509NameType]::SimpleName, $false)
if ($Publisher -ne 'Pyrsys B.V.') { throw "Unexpected Inno Setup publisher: $Publisher" }
$InstallPath = Join-Path $Directory 'compiler'
$Process = Start-Process -FilePath $Installer -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', "/DIR=`"$InstallPath`"") -Wait -PassThru
if ($Process.ExitCode -ne 0) { throw "Inno Setup installer exited with $($Process.ExitCode)" }
$Iscc = Join-Path $InstallPath 'ISCC.exe'
if (-not (Test-Path -LiteralPath $Iscc -PathType Leaf)) { throw 'ISCC.exe missing after installer completion.' }
"HUDA_ISCC=$Iscc" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
Write-Host "Verified Inno Setup $Version compiler installed for this disposable runner."
