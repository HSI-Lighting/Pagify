<#
.SYNOPSIS
    Downloads the pinned PDFium prebuilt binaries into app/src/main/jniLibs.

.DESCRIPTION
    The version is pinned, not "latest", and the pin is not arbitrary:
    pdfium-render 0.9.3's `pdfium_latest` feature binds the chromium/7881 API
    surface. Pairing those bindings with a different PDFium build risks the
    dynamic loader failing to resolve a symbol at the first render rather than at
    link time. If you bump pdfium-render, bump $PdfiumTag to match its newest
    `pdfium_NNNN` feature.

    The resulting .so files are checked into git on purpose — the build cannot
    reproduce them locally, and a fresh clone should just build.

.EXAMPLE
    pwsh tools/fetch_pdfium.ps1
#>
[CmdletBinding()]
param(
    [string]$PdfiumTag = "chromium/7881",
    [string]$JniLibsDir = (Join-Path $PSScriptRoot "..\app\src\main\jniLibs")
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

# PDFium's own naming on the left, Android's ABI names on the right.
$abiMap = @{
    "arm64" = "arm64-v8a"
    "arm"   = "armeabi-v7a"
    "x64"   = "x86_64"
}

$encodedTag = $PdfiumTag -replace "/", "%2F"
$staging = Join-Path ([System.IO.Path]::GetTempPath()) "pagify-pdfium-$([guid]::NewGuid())"
New-Item -ItemType Directory -Force -Path $staging | Out-Null

try {
    foreach ($pdfiumArch in $abiMap.Keys) {
        $abi = $abiMap[$pdfiumArch]
        $url = "https://github.com/bblanchon/pdfium-binaries/releases/download/$encodedTag/pdfium-android-$pdfiumArch.tgz"
        $archive = Join-Path $staging "$pdfiumArch.tgz"

        Write-Host "Fetching $abi from $PdfiumTag ..."
        curl.exe -sSL --fail -o $archive $url
        if ($LASTEXITCODE -ne 0) { throw "download failed for $abi ($url)" }

        $extracted = Join-Path $staging $pdfiumArch
        New-Item -ItemType Directory -Force -Path $extracted | Out-Null
        tar -xzf $archive -C $extracted
        if ($LASTEXITCODE -ne 0) { throw "could not extract $archive" }

        $dest = Join-Path $JniLibsDir $abi
        New-Item -ItemType Directory -Force -Path $dest | Out-Null
        Copy-Item (Join-Path $extracted "lib\libpdfium.so") (Join-Path $dest "libpdfium.so") -Force

        $version = (Get-Content (Join-Path $extracted "VERSION")) -join " "
        $sizeMb = [math]::Round((Get-Item (Join-Path $dest "libpdfium.so")).Length / 1MB, 2)
        Write-Host "  -> $abi  $sizeMb MB  ($version)"
    }

    Write-Host ""
    Write-Host "Done. Verify 16 KB page alignment for the 64-bit ABIs with:"
    Write-Host '  llvm-readelf -l app/src/main/jniLibs/arm64-v8a/libpdfium.so | Select-String LOAD'
    Write-Host "(the alignment column must read 0x4000 for arm64-v8a and x86_64)"
}
finally {
    Remove-Item $staging -Recurse -Force -ErrorAction SilentlyContinue
}
