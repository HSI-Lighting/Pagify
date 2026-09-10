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

if ($env:SIGNING_CERT) {
    # The DLL first, then the exe. Same rule as macOS.
    & signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 `
        /f $env:SIGNING_CERT /p $env:SIGNING_PASSWORD (Join-Path $Out "pdfium.dll")
    & signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 `
        /f $env:SIGNING_CERT /p $env:SIGNING_PASSWORD (Join-Path $Out "Pagify.exe")
} else {
    Write-Host "SIGNING_CERT not set - built unsigned. SmartScreen will warn on any other machine."
}

Write-Host "==> $Out"
