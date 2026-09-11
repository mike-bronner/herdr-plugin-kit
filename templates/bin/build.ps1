# Gets this plugin's binary on Windows: fetch a published one, or compile it.
#
# SYNCED FROM herdr-plugin-kit. Edit templates/bin/build.ps1 in the kit, never
# this copy.
#
# ================================ WARNING =================================
#
# NOTHING IN THIS FILE HAS EVER BEEN RUN, and it has not even been parsed for
# syntax. Nobody on this project has Windows hardware and PowerShell is not
# installed on the machine this was written on. See bin/common.ps1.
#
# ==========================================================================
#
# It exists because ["sh", "bin/build"] cannot run on Windows at all. A Windows
# user installs the plugin, the sh shim never executes, and the release asset
# built for that user is unreachable. Shipping six target triples and holding
# these shims would produce artifacts nobody can install, which is why the two
# were decided together. SCOPE.md section 10.1.
#
# Herdr's per-item platforms override declares it beside the shell one:
#
#     [[build]]
#     command = ["sh", "bin/build", "--install"]
#     platforms = ["linux", "macos"]
#
#     [[build]]
#     command = ["powershell", "-File", "bin/build.ps1", "--install"]
#     platforms = ["windows"]
#
# It is deliberately plainer than bin/build. There is no progress display here:
# the seam that will become a Herdr popup lives in bin/progress, which is
# sh-only, and adding a second unverified drawing path to an unverified file
# would be the worst place in this repository to put one.

param([switch] $Install)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'common.ps1')

# ---- Which way round -----------------------------------------------------

# Asks Herdr what kind of install this is, and fails closed to 'local'.
#
# 'local' means somebody's working tree, so it compiles. Everything that is not
# plainly 'github' reaches that same answer: no HERDR_BIN_PATH, an unreachable
# socket, or a response that will not parse. Being wrong that way costs a slow
# compile; being wrong the other way hands a developer who asked to compile a
# binary somebody else built.
function Get-SourceKind {
    if (-not $env:HERDR_BIN_PATH -or -not (Test-Path -LiteralPath $env:HERDR_BIN_PATH)) {
        return 'local'
    }
    $id = Get-PluginId
    if (-not $id) { return 'local' }
    $json = & $env:HERDR_BIN_PATH plugin list --plugin $id --json 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $json) { return 'local' }
    if ($json -match '"source":\{"kind":"github"\}') { return 'github' }
    return 'local'
}

function Test-ShouldFetch {
    if (Test-Path -LiteralPath (Join-Path $PluginRoot $OverrideFile)) {
        Write-Note "$OverrideFile is in the plugin root, so this tree compiles"
        return $false
    }
    # The manifest flag names the calling context. [[build]] fires only on
    # `herdr plugin install`, never on `herdr plugin link`, so the install is
    # 'github' by construction there.
    if ($Install) { return $true }
    if ((Get-SourceKind) -eq 'github') { return $true }
    Write-Note 'Herdr reports this as a local install, so this tree compiles'
    return $false
}

# ---- Fetching ------------------------------------------------------------

# Downloads the published binary and moves it into place only once its
# published checksum has matched.
#
# The move is the gate. Verification happens beside the target under a
# different name, so an unverified file never occupies the path the launcher
# runs, and no failure mode can execute one.
function Invoke-Fetch {
    $url = Get-AssetUrl
    if (-not $url) {
        Write-Note 'no published binary matches this platform and this commit'
        return $false
    }

    $releaseDir = Split-Path -Parent $Binary
    New-Item -ItemType Directory -Force -Path $releaseDir | Out-Null
    $part = "$Binary.part"
    $partSum = "$Binary.part.sha256"

    try {
        Invoke-WebRequest -Uri $url -OutFile $part -UseBasicParsing -TimeoutSec 120
        Invoke-WebRequest -Uri "$url.sha256" -OutFile $partSum -UseBasicParsing -TimeoutSec 120
    } catch {
        Write-Note "nothing could be downloaded from ${url}: $($_.Exception.Message)"
        Remove-Item -LiteralPath $part, $partSum -Force -ErrorAction SilentlyContinue
        return $false
    }

    $expected = ((Get-Content -LiteralPath $partSum -First 1) -split '\s+')[0].ToLowerInvariant()
    if ($expected -notmatch '^[0-9a-f]{64}$') {
        Write-Note 'the published checksum is not a sha256 digest'
        Remove-Item -LiteralPath $part, $partSum -Force -ErrorAction SilentlyContinue
        return $false
    }

    $actual = (Get-FileHash -LiteralPath $part -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) {
        # Loud, and never cached. A mismatch still falls back to compiling,
        # because building from the cloned source is safe, but the bad artifact
        # must never be reused.
        Write-Note 'SECURITY: the downloaded binary does not match its published checksum.'
        Write-Note "  url:      $url"
        Write-Note "  expected: $expected"
        Write-Note "  actual:   $actual"
        Write-Note 'The download has been deleted. Compiling from source instead.'
        Remove-Item -LiteralPath $part, $partSum -Force -ErrorAction SilentlyContinue
        return $false
    }

    Move-Item -LiteralPath $part -Destination $Binary -Force
    Write-Provenance (Get-ManifestVersion) ($url -split '/')[-1] $actual $url
    Remove-Item -LiteralPath $partSum -Force -ErrorAction SilentlyContinue
    return $true
}

# ---- Compiling -----------------------------------------------------------

function Find-Cargo {
    $onPath = Get-Command cargo -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }
    foreach ($candidate in @(
            $env:CARGO,
            (Join-Path (Join-Path ($env:CARGO_HOME ?? (Join-Path $env:USERPROFILE '.cargo')) 'bin') 'cargo.exe'),
            (Join-Path (Join-Path (Join-Path $env:USERPROFILE '.cargo') 'bin') 'cargo.exe')
        )) {
        if ($candidate -and (Test-Path -LiteralPath $candidate)) { return $candidate }
    }
    return ''
}

function Invoke-Compile {
    $cargo = Find-Cargo
    if (-not $cargo) {
        Write-Note 'no Rust toolchain here, and no published binary could be used.'
        Stop-WithNote "install Rust from https://rustup.rs, then run ``cargo build --release`` in $PluginRoot"
    }
    & $cargo build --release --manifest-path (Join-Path $PluginRoot 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) {
        Stop-WithNote "cargo build --release failed in $PluginRoot"
    }
    # The other half of Write-Provenance. A compiled binary keeping a note from
    # an earlier fetch would report itself as downloaded, and would then tell
    # somebody holding a stale binary to reinstall rather than rebuild.
    Remove-Item -LiteralPath $Mark -Force -ErrorAction SilentlyContinue
}

# ---- The order -----------------------------------------------------------

# Is a build needed at all? First, so the common case costs no network call.
if (-not (Test-NeedsBuild)) { exit 0 }

if (Test-ShouldFetch) {
    Write-Note 'downloading the build published for this platform'
    if (Invoke-Fetch) { exit 0 }
    Write-Note 'compiling from source instead'
}

Invoke-Compile
