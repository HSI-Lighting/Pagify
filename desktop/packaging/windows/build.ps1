# Build and sign Pagify for Windows.
#
# `pdfium.dll` goes beside the executable — the first place
# pagify_shell::pdfium looks — and is signed before the installer that wraps
# it, for the same reason the macOS script signs the nested dylib first: a
# signature over a container is a hash of its contents.
$ErrorActionPreference = "Stop"

$Root = Resolve-Path "$PSScriptRoot\..\.."
$Dll  = Join-Path $Root "third_party\pdfium\pdfium-win-x64\bin\pdfium.dll"
$Out  = Join-Path $Root "target\Pagify"

if (-not (Test-Path $Dll)) {
    throw "No Windows PDFium at $Dll - run tools/fetch_pdfium.sh (or the .ps1) first."
}

cargo build --release -p pagify_app
if ($LASTEXITCODE -ne 0) { throw "build failed" }

Remove-Item -Recurse -Force $Out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $Out | Out-Null
Copy-Item (Join-Path $Root "target\release\pagify_app.exe") (Join-Path $Out "Pagify.exe")
Copy-Item $Dll (Join-Path $Out "pdfium.dll")

# The certificate comes from the user's certificate store, named by its
# thumbprint — never as a PFX file with its password on the command line,
# where every process list and shell history could read it (security audit
# L2). Put it in the store once, with the password typed at a prompt rather
# than passed:
#
#     Import-PfxCertificate -FilePath .\signing.pfx -CertStoreLocation Cert:\CurrentUser\My `
#         -Password (Read-Host -AsSecureString "PFX password")
#     Get-ChildItem Cert:\CurrentUser\My | Format-Table Thumbprint, Subject
#
# and set SIGNING_THUMBPRINT to the thumbprint it prints.
if ($env:SIGNING_THUMBPRINT) {
    # The DLL first, then the exe. Same rule as macOS.
    & signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 `
        /sha1 $env:SIGNING_THUMBPRINT (Join-Path $Out "pdfium.dll")
    if ($LASTEXITCODE -ne 0) { throw "signing pdfium.dll failed" }
    & signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 `
        /sha1 $env:SIGNING_THUMBPRINT (Join-Path $Out "Pagify.exe")
    if ($LASTEXITCODE -ne 0) { throw "signing Pagify.exe failed" }
} elseif ($env:SIGNING_CERT -or $env:SIGNING_PASSWORD) {
    throw "SIGNING_CERT/SIGNING_PASSWORD are no longer used: import the PFX into the certificate store and set SIGNING_THUMBPRINT (see the comment above)."
} else {
    Write-Host "SIGNING_THUMBPRINT not set - built unsigned. SmartScreen will warn on any other machine."
}

Write-Host "==> $Out"
