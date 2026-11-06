# Build and sign Pagify for Windows.
#
# `pdfium.dll` goes beside the executable — the first place
# pagify_shell::pdfium looks — and is signed before the installer that wraps
# it, for the same reason the macOS script signs the nested dylib first: a
# signature over a container is a hash of its contents.
#
# `-Publish` additionally copies the signed build into the Dropbox folder
# every PC this runs on already syncs — see `UPDATE_FOLDER` in main.rs, which
# is what actually looks for it — and, signed with Pagify's update key, to
# GitHub (HSI-Lighting/release), which every installed Pagify also reads (see
# `update.rs`). Opt-in and separate from a plain build: a routine local
# rebuild must never silently push itself out to every other machine.
param(
    [switch]$Publish
)
$ErrorActionPreference = "Stop"

$Root = Resolve-Path "$PSScriptRoot\..\.."
$Dll  = Join-Path $Root "third_party\pdfium\pdfium-win-x64\bin\pdfium.dll"
$Out  = Join-Path $Root "target\Pagify"
# `pagify_shell::models` loads these from disk ("beside the executable… is
# what an installed app uses") — unlike the fonts and dictionary, which are
# `include_bytes!`/`include_str!`'d into the exe itself. Missing from every
# build this script produced until now: OCR worked on a dev machine only
# because `models.rs`'s own fallback #2 finds the checkout's own
# `third_party\ocr` when run via `cargo run`, which masked this exactly the
# way a dev machine's already-installed VC++ Redistributable masked the
# VCRUNTIME gap.
$OcrModels = @(
    (Join-Path $Root "third_party\ocr\text-detection.rten"),
    (Join-Path $Root "third_party\ocr\text-recognition.rten")
)

if (-not (Test-Path $Dll)) {
    throw "No Windows PDFium at $Dll - run tools/fetch_pdfium.sh (or the .ps1) first."
}
foreach ($Model in $OcrModels) {
    if (-not (Test-Path $Model)) {
        throw "No OCR model at $Model - it should already be checked out under third_party/ocr; 'verify third_party' below should have caught this first."
    }
}

# The same three release gates macOS runs before it will build (security
# audit M-6) — checksums, advisories, no sockets — via Git Bash, which
# building this workspace on Windows already requires for fetch_pdfium.sh.
# Not always on PATH even when installed, so the well-known install location
# is a fallback rather than a second requirement.
$Bash = Get-Command bash -ErrorAction SilentlyContinue
if (-not $Bash) {
    $Fallback = "$env:ProgramFiles\Git\bin\bash.exe"
    if (Test-Path $Fallback) { $Bash = $Fallback } else { $Bash = $null }
}
if (-not $Bash) {
    throw "Git Bash is required to run the release gates (tools/*.sh) and was not found on PATH or at $env:ProgramFiles\Git\bin\bash.exe."
}
function Invoke-Gate($Name, $Script) {
    Write-Host "==> $Name"
    & $Bash $Script
    if ($LASTEXITCODE -ne 0) { throw "$Name failed" }
}
Invoke-Gate "verify third_party" (Join-Path $Root "tools\verify_third_party.sh")
Invoke-Gate "audit dependencies" (Join-Path $Root "tools\audit.sh")
Invoke-Gate "no sockets" (Join-Path $Root "tools\no_sockets.sh")

# `--locked`: the gates above just ran against `Cargo.lock` as it stands, so
# the build has to use exactly that lock, not update it and build something
# the gate never saw. Found by audit.
cargo build --release -p pagify_app --locked
if ($LASTEXITCODE -ne 0) { throw "build failed" }

Remove-Item -Recurse -Force $Out -ErrorAction SilentlyContinue
# `-Force`: a Pagify started from this folder is that process's working
# directory, and Windows will not delete a folder in use - the files go, the
# folder stays, and a bare `New-Item` then stopped every publish.
New-Item -ItemType Directory -Path $Out -Force | Out-Null
Copy-Item (Join-Path $Root "target\release\pagify_app.exe") (Join-Path $Out "Pagify.exe")
Copy-Item $Dll (Join-Path $Out "pdfium.dll")
New-Item -ItemType Directory -Path (Join-Path $Out "ocr") -Force | Out-Null
foreach ($Model in $OcrModels) {
    Copy-Item $Model (Join-Path $Out "ocr")
}

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

if ($Publish) {
    $UpdateFolder = "D:\Dropbox\YASEEN\pagify desktop"
    $CargoToml = Get-Content (Join-Path $Root "Cargo.toml") -Raw
    if ($CargoToml -notmatch '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"') {
        throw "could not find a [workspace.package] version in $Root\Cargo.toml"
    }
    $Version = $Matches[1]

    New-Item -ItemType Directory -Path $UpdateFolder -Force | Out-Null
    # Loose files, not the zip below: `spawn_update_script` in main.rs copies
    # `Pagify.exe`/`pdfium.dll` from these exact names and does the actual
    # self-update of an *already-installed* copy. Breaking that shape here
    # would silently stop every machine that already has Pagify from ever
    # updating itself again.
    Copy-Item (Join-Path $Out "Pagify.exe") (Join-Path $UpdateFolder "Pagify.exe") -Force
    Copy-Item (Join-Path $Out "pdfium.dll") (Join-Path $UpdateFolder "pdfium.dll") -Force
    # Contents (`ocr\*`) into an already-created destination, not the `ocr`
    # folder itself: `Copy-Item -Recurse` onto a destination that already
    # exists nests the source folder one level deeper instead of merging its
    # contents, which this folder — never deleted between publishes, unlike
    # the fresh `$Out` above — would hit on the second run, landing the
    # models at `ocr\ocr\*.rten` where nothing loads them from.
    New-Item -ItemType Directory -Path (Join-Path $UpdateFolder "ocr") -Force | Out-Null
    Copy-Item (Join-Path $Out "ocr\*") (Join-Path $UpdateFolder "ocr") -Force
    # ASCII, not the default UTF-8: Windows PowerShell 5.1's `-Encoding utf8`
    # writes a byte-order mark, which would land as a stray leading character
    # `read_update_manifest` never asked to strip. A version string is plain
    # digits and dots either way.
    Set-Content -Path (Join-Path $UpdateFolder "version.txt") -Value $Version -NoNewline -Encoding ascii

    # The zip is for a *fresh* install — someone with no Pagify on the
    # machine yet, who the loose files and `version.txt` above mean nothing
    # to. One file, everything it needs inside (`$Out` already has it all —
    # same source the loose copy above just used), extracts to a "Pagify"
    # folder rather than scattering loose files wherever it's unzipped, and
    # is overwritten every publish so it is never a version behind.
    $Zip = Join-Path $UpdateFolder "Pagify.zip"
    Remove-Item $Zip -ErrorAction SilentlyContinue
    Compress-Archive -Path $Out -DestinationPath $Zip

    Write-Host "==> published $Version to $UpdateFolder (loose files + $Zip)"

    # GitHub: the same zip, signed with Pagify's update key, as release
    # `pagify-v<ver>` of HSI-Lighting/release, and `pagify/latest.json` there
    # naming it - the feed every installed Pagify reads beside this folder
    # (`update.rs`), as every HSI app reads its own (that repo's README). The
    # key is kept on the PC that publishes (`examples/make_update_key.rs`);
    # PAGIFY_UPDATE_KEY names another place. Its password, if it has one,
    # comes from PAGIFY_UPDATE_KEY_PASSWORD, never this command line.
    $Key = if ($env:PAGIFY_UPDATE_KEY) { $env:PAGIFY_UPDATE_KEY } else { "E:\KEYS\Pagify\pagify-updater.key" }
    if (-not (Test-Path $Key)) {
        Write-Warning "no Pagify update key at $Key (PAGIFY_UPDATE_KEY names another place): published to Dropbox only, NOT to GitHub"
        return
    }
    $Asset = "pagify_${Version}_windows.zip"
    $Signed = Join-Path $env:TEMP $Asset
    Copy-Item $Zip $Signed -Force
    $Signature = & cargo run -q --release --locked -p pagify_app --example sign_update -- $Key $Signed
    if ($LASTEXITCODE -ne 0 -or -not $Signature) { throw "signing $Asset failed" }

    # Only the HSI-Lighting account can write the release repo: its token, as git keeps it.
    $env:GIT_TERMINAL_PROMPT = "0"; $env:GCM_INTERACTIVE = "never"
    $Token = ("protocol=https`nhost=github.com`nusername=HSI-Lighting`n`n" | git credential fill | Where-Object { $_ -like "password=*" }) -replace '^password=', ''
    if (-not $Token) { throw "git holds no GitHub credential for HSI-Lighting" }
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    $Api = "https://api.github.com/repos/HSI-Lighting/release"
    $Headers = @{ Authorization = "Bearer $Token"; Accept = "application/vnd.github+json" }
    $Tag = "pagify-v$Version"
    try { $Release = Invoke-RestMethod "$Api/releases/tags/$Tag" -Headers $Headers }
    catch {
        $New = @{ tag_name = $Tag; target_commitish = "main"; name = "Pagify $Version"; body = "Pagify $Version for Windows: unzip, or let Pagify update itself." }
        $Release = Invoke-RestMethod "$Api/releases" -Method Post -Headers $Headers -ContentType "application/json" -Body ($New | ConvertTo-Json)
    }
    # Publishing the same version again replaces its zip.
    foreach ($Old in @($Release.assets | Where-Object { $_.name -eq $Asset })) {
        Invoke-RestMethod "$Api/releases/assets/$($Old.id)" -Method Delete -Headers $Headers | Out-Null
    }
    Invoke-RestMethod "https://uploads.github.com/repos/HSI-Lighting/release/releases/$($Release.id)/assets?name=$Asset" `
        -Method Post -Headers $Headers -ContentType "application/zip" -InFile $Signed | Out-Null
    $Url = "https://github.com/HSI-Lighting/release/releases/download/$Tag/$Asset"

    $Feed = [ordered]@{
        version   = $Version
        notes     = "Pagify $Version"
        pub_date  = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
        platforms = [ordered]@{ "windows-x86_64" = [ordered]@{ signature = "$Signature"; url = $Url } }
    } | ConvertTo-Json -Depth 4
    $FeedApi = "$Api/contents/pagify/latest.json"
    $Put = @{ message = "Pagify $Version (windows)"; branch = "main"; content = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($Feed)) }
    try { $Put.sha = (Invoke-RestMethod "$FeedApi`?ref=main" -Headers $Headers).sha } catch { }
    Invoke-RestMethod $FeedApi -Method Put -Headers $Headers -ContentType "application/json" -Body ($Put | ConvertTo-Json) | Out-Null
    Remove-Item $Signed
    Write-Host "==> published $Version to GitHub: $Url (feed pagify/latest.json)"
}
