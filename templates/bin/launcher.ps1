# Runs this plugin's binary on Windows, building it first when it is behind.
#
# SYNCED FROM herdr-plugin-kit. Edit templates/bin/launcher.ps1 in the kit,
# never this copy.
#
# ================================ WARNING =================================
#
# NOTHING IN THIS FILE HAS EVER BEEN RUN, and it has not even been parsed for
# syntax. Nobody on this project has Windows hardware and PowerShell is not
# installed on the machine this was written on. See bin/common.ps1.
#
# ==========================================================================
#
# Declared beside the shell launcher with Herdr's per-item platforms override:
#
#     [[panes]]
#     command = ["sh", "bin/launcher"]
#     platforms = ["linux", "macos"]
#
#     [[panes]]
#     command = ["powershell", "-File", "bin/launcher.ps1"]
#     platforms = ["windows"]

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'common.ps1')

$Forwarded = $args

# Deliberately before the staleness check, and it must stay there. --version
# exists to diagnose a binary that disagrees with its manifest, and a rebuild
# running first would replace that binary with a current one. The command would
# then report agreement every time, and could never show the state it was
# written to show.
if ($Forwarded.Count -gt 0 -and $Forwarded[0] -eq '--version') {
    if (-not (Test-Path -LiteralPath $Binary)) {
        Stop-WithNote "no $BinaryName binary at $Binary"
    }
    $env:HERDR_PLUGIN_ROOT = $PluginRoot
    & $Binary --version
    exit $LASTEXITCODE
}

if (Test-NeedsBuild) {
    $env:HERDR_PLUGIN_ROOT = $PluginRoot
    & powershell -NoProfile -File (Join-Path $PSScriptRoot 'build.ps1')
    if ($LASTEXITCODE -ne 0) {
        if (Test-Path -LiteralPath $Binary) {
            # A stale binary that runs beats no binary at all, and saying so is
            # the only way somebody finds out why their edit did nothing.
            Write-Note "the build failed; running the $BinaryName already built, which may be stale"
        } else {
            Stop-WithNote "no $BinaryName binary at $Binary and it could not be built"
        }
    }
}

if (-not (Test-Path -LiteralPath $Binary)) {
    Stop-WithNote "no $BinaryName binary at $Binary"
}

& $Binary @Forwarded
exit $LASTEXITCODE
