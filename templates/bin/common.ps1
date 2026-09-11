# Shared by bin/build.ps1 and bin/launcher.ps1. Dot-sourced, never run.
#
# SYNCED FROM herdr-plugin-kit. Edit templates/bin/common.ps1 in the kit,
# never this copy.
#
# ================================ WARNING =================================
#
# NOTHING IN THIS FILE HAS EVER BEEN RUN.
#
# Not once, and not by anybody. Nobody on this project has Windows hardware,
# and PowerShell is not installed on the machine this was written on, so this
# file has not even been parsed for syntax. The kit's README says Windows is
# "compile-verified only", which is a claim about the Rust code that CI builds.
# PowerShell has no compiler and no CI job, so this file does not reach even
# that bar.
#
# Treat a bug here as new information, never as a regression.
#
# It is deliberately the plainest thing that can work. Its shell counterpart
# draws a progress display and classifies four kinds of download failure; this
# does neither, because unverified code should be small. Every line here is one
# nobody can test.
#
# ==========================================================================
#
# It is a mirror of bin/common, function for function, so the two can be read
# side by side. templates/test_templates.py asserts the two lists agree, which
# is the only check available without a Windows machine.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# ---- Where we are --------------------------------------------------------

$PluginRoot = if ($env:HERDR_PLUGIN_ROOT) {
    $env:HERDR_PLUGIN_ROOT
} else {
    Split-Path -Parent (Split-Path -Parent $PSCommandPath)
}

$Manifest = Join-Path $PluginRoot 'herdr-plugin.toml'
$CargoManifest = Join-Path $PluginRoot 'Cargo.toml'

# Has to agree with herdr_plugin_kit::version::PROVENANCE_SUFFIX, which reads
# the file the fetch path writes.
$ProvenanceSuffix = '.download'

# The developer override. A file rather than an environment variable, because
# Herdr launches these scripts itself and a developer's shell never reaches
# them. See bin/common for the whole reason.
$OverrideFile = 'BUILD_FROM_SOURCE'

# ---- THE ONE OPEN WINDOWS QUESTION ---------------------------------------

# Does a Windows release asset's name carry '.exe'?
#
# UNVERIFIED. Recommended, not confirmed, and this is the *consuming* half of
# the answer.
#
# SCOPE.md section 14.2 holds the question and now records the recommendation:
# a file without this extension is not executable on Windows, and somebody
# downloading from the releases page should get something that runs. Nobody on
# this project has Windows hardware, so nothing has produced or consumed one of
# these names and nothing here is a measurement.
#
# The producing half is WINDOWS_ASSET_EXTENSION in tools/plugin_gate.py, which
# is what .github/workflows/plugin-release.yml publishes. Those two assignments
# are the only places in this repository that decide this name, and
# tools/test_plugin_gate.py fails if they ever disagree — a producer and a
# consumer disagreeing here is a 404 and a silent compile on every Windows
# install. Reversing the recommendation is one line in each.
#
# No shell template names a Windows asset at all: bin/common's platform()
# answers only macos and linux, and refuses everything else rather than
# guessing. templates/test_templates.py asserts that too.
#
# Being wrong here costs a 404 and a compile, never a wrong binary, because the
# checksum gate below the fetch does not care what the file was called.
$AssetNameExtension = '.exe'

# ---- Reading the plugin's own files --------------------------------------

# Reads one key from a TOML file's top-level table.
#
# Stopping at the first section header is the point. herdr-plugin.toml carries
# further 'id' keys in [[panes]] and [[actions]] entries, and a line-anchored
# search returns the wrong one in two of the three plugins, silently.
function Get-TomlTopLevel {
    param([string] $Path, [string] $Key)

    if (-not (Test-Path -LiteralPath $Path)) { return '' }
    foreach ($line in Get-Content -LiteralPath $Path) {
        if ($line -match '^\s*\[') { break }
        $found = [regex]::Match($line, '^\s*' + [regex]::Escape($Key) + '\s*=\s*"([^"]*)"')
        if ($found.Success) { return $found.Groups[1].Value }
    }
    return ''
}

# Cargo.toml's [[bin]] name, not [package] name. Exactly one is required:
# guessing between two would fetch one asset and run another.
function Get-BinaryName {
    param([string] $Path)

    if (-not (Test-Path -LiteralPath $Path)) { return '' }
    $inside = $false
    $names = @()
    foreach ($line in Get-Content -LiteralPath $Path) {
        if ($line -match '^\s*\[\[bin\]\]\s*$') { $inside = $true; continue }
        if ($line -match '^\s*\[') { $inside = $false; continue }
        if (-not $inside) { continue }
        $found = [regex]::Match($line, '^\s*name\s*=\s*"([^"]*)"')
        if ($found.Success) { $names += $found.Groups[1].Value }
    }
    if ($names.Count -eq 1) { return $names[0] }
    return ''
}

# The plugin's own name. Herdr ids are '<namespace>.<slug>', and the slug names
# the plugin rather than its binary.
function Get-PluginSlug {
    $id = Get-TomlTopLevel $Manifest 'id'
    if (-not $id) { return '' }
    return $id.Substring($id.LastIndexOf('.') + 1)
}

function Get-ManifestVersion { return Get-TomlTopLevel $Manifest 'version' }

function Get-PluginId { return Get-TomlTopLevel $Manifest 'id' }

# ---- Saying things -------------------------------------------------------

$Slug = Get-PluginSlug
if (-not $Slug) { $Slug = 'herdr-plugin' }

function Write-Note {
    param([string] $Message)
    [Console]::Error.WriteLine("${Slug}: $Message")
}

function Stop-WithNote {
    param([string] $Message)
    Write-Note $Message
    exit 1
}

# The counterpart of bin/common's can_draw, and the same rule: never draw where
# nothing can render it. A Herdr [[startup]] command is handed a pipe, not a
# terminal, and an escape sequence written there lands in the server log as
# bytes nobody can read.
#
# Windows has no TERM, so the question reduces to whether stderr is redirected.
# Nothing here draws anyway: this decides where cargo's own output goes.
function Test-CanDraw {
    return -not [Console]::IsErrorRedirected
}

# ---- What the compiler reads ---------------------------------------------

# The same list as bin/common's COMPILER_INPUTS and the kit's build stamp, for
# the same reason: three questions ask it, and letting them differ makes the
# stamp and the fetch disagree about one tree.
#
# 'rust-toolchain' and 'rust-toolchain.toml' are both here and neither is
# redundant. Git pathspecs match whole path components.
$CompilerInputs = @(
    'src', 'Cargo.toml', 'Cargo.lock', 'build.rs',
    '.cargo', 'rust-toolchain', 'rust-toolchain.toml'
)

# Whether anything the compiler reads is newer than the binary — or exactly as
# old as it.
#
# 🪤 The second half is a real defect its shell counterpart shipped with for two
# stages, found on Linux 2026-09-11 and fixed on both sides together. Two files
# written in the same clock tick can carry an identical timestamp, and a
# strictly-newer comparison then reads an edited source as current and runs a
# stale binary in silence.
#
# Equality means the filesystem cannot say which came first, which is not the
# same as "the binary is current". It fails closed, like every other
# unanswerable question here.
#
# ⚠️ This side is unrun like the rest of this file. It is -ge rather than -gt
# because the rule is the rule, not because anybody has seen NTFS tie.
function Test-CompilerInputNotOlderThan {
    param([string] $Path)

    if (-not (Test-Path -LiteralPath $Path)) { return $true }
    $stamp = (Get-Item -LiteralPath $Path).LastWriteTimeUtc
    foreach ($input in $CompilerInputs) {
        $candidate = Join-Path $PluginRoot $input
        if (-not (Test-Path -LiteralPath $candidate)) { continue }
        $newer = Get-ChildItem -LiteralPath $candidate -Recurse -File -ErrorAction SilentlyContinue |
            Where-Object { $_.LastWriteTimeUtc -ge $stamp } |
            Select-Object -First 1
        if ($newer) { return $true }
        $itself = Get-Item -LiteralPath $candidate
        if (-not $itself.PSIsContainer -and $itself.LastWriteTimeUtc -ge $stamp) { return $true }
    }
    return $false
}

# ---- The binary, and whether it is current -------------------------------

$BinaryName = Get-BinaryName $CargoManifest
if (-not $BinaryName) {
    Stop-WithNote 'Cargo.toml declares no single [[bin]] name, so there is no binary to build or fetch'
}

# Cargo always writes an .exe on Windows.
#
# This is NOT the open question above. That one is about what a release asset
# is *called* on the server; this is about what cargo puts on disk, which is
# settled and always has been.
$Binary = Join-Path (Join-Path (Join-Path $PluginRoot 'target') 'release') "$BinaryName.exe"
$Mark = "$Binary$ProvenanceSuffix"

function Get-NoteValue {
    param([string] $Key)

    if (-not (Test-Path -LiteralPath $Mark)) { return '' }
    foreach ($line in Get-Content -LiteralPath $Mark) {
        $found = [regex]::Match($line, '^' + [regex]::Escape($Key) + '=(.*)$')
        if ($found.Success) { return $found.Groups[1].Value }
    }
    return ''
}

# A downloaded binary and a compiled one go stale for different reasons.
#
# A compiled binary belongs to the source beside it, so timestamps answer. A
# downloaded one belongs to a release, and on any commit past that release the
# source is permanently newer — so timestamps would say "rebuild" forever, on
# the one machine with no toolchain to rebuild with. An unreadable version
# fails closed and builds.
function Test-NeedsBuild {
    if (-not (Test-Path -LiteralPath $Binary)) { return $true }
    if (Test-Path -LiteralPath $Mark) {
        $declared = Get-ManifestVersion
        if (-not $declared) { return $true }
        return $declared -ne (Get-NoteValue 'version')
    }
    return Test-CompilerInputNotOlderThan $Binary
}

# ---- Naming the published asset ------------------------------------------

# Windows only. Its shell counterpart answers macos and linux and refuses
# everything else, which is what keeps the open .exe question to this one file.
function Get-Platform {
    switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture) {
        'Arm64' { return 'windows-arm64' }
        'X64' { return 'windows-x64' }
        default { return '' }
    }
}

function Get-RepoSlug {
    $url = & git -C $PluginRoot remote get-url origin 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $url) { return '' }
    $found = [regex]::Match($url, 'github\.com[:/](.+?)(?:\.git)?\s*$')
    if (-not $found.Success) { return '' }
    $slug = $found.Groups[1].Value
    if ($slug -notmatch '^[0-9A-Za-z._-]+/[0-9A-Za-z._-]+$') { return '' }
    return $slug
}

# The first twelve characters of HEAD, but only when this checkout could have
# produced a published binary: its own repository root, clean in every
# compiler-read path, and a hex commit.
function Get-ReleasedCommit {
    $top = & git -C $PluginRoot rev-parse --show-toplevel 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $top) { return '' }
    $dirty = & git -C $PluginRoot status --porcelain -- $CompilerInputs 2>$null
    if ($LASTEXITCODE -ne 0 -or $dirty) { return '' }
    $commit = & git -C $PluginRoot rev-parse HEAD 2>$null
    if ($LASTEXITCODE -ne 0 -or $commit -notmatch '^[0-9a-f]{12,}$') { return '' }
    return $commit.Substring(0, 12)
}

# The URL itself asserts that the binary was built from the source in this
# folder. A checkout one commit past the tag asks for a file that does not
# exist, gets a 404, and compiles, with no network call to resolve a tag.
function Get-AssetUrl {
    $platform = Get-Platform
    if (-not $platform) { return '' }
    $version = Get-ManifestVersion
    if (-not $version) { return '' }
    $slug = Get-RepoSlug
    if (-not $slug) { return '' }
    $commit = Get-ReleasedCommit
    if (-not $commit) { return '' }
    $asset = "$BinaryName-$platform-$commit$AssetNameExtension"
    return "https://github.com/$slug/releases/download/$version/$asset"
}

# Records how this binary arrived, beside the binary it describes.
#
# Without it a fetched binary reports itself as built from source, because that
# is what the absence of this file means. The four keys are the shape
# herdr_plugin_kit::version::provenance_of reads.
function Write-Provenance {
    param([string] $Version, [string] $Asset, [string] $Sha256, [string] $Url)

    $text = "version=$Version`nasset=$Asset`nsha256=$Sha256`nurl=$Url`n"
    [System.IO.File]::WriteAllText($Mark, $text)
}
