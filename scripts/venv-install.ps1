# venv-install.ps1 <requirements file> [--dry-run]: installs pinned wheels from a tracked, unmodified
# shared/requirements-*.txt into the one shared venv (shared/venv). An allow rule points at this script, so it is a
# SAFETY BOUNDARY: it REFUSES BY DEFAULT (exit 2, a reason on stderr, before any side effect) and installs only when
# every check passes. The root is the parent of this script's folder; there is no override. It never pushes, forces,
# resets, deletes files or history, reads .env or edits .claude/.
#
# Checks, all of which must pass:
#  1. Exactly one argument (plus an optional trailing --dry-run): a relative path `shared/requirements-<name>.txt`
#     (<name> is letters, digits, '.', '_', '-'), no '~' (8.3 short names), resolving inside <root>/shared, with no
#     junction or symlink on the way, naming an existing file.
#  2. The root is a git checkout (its top level), the file is tracked (`git ls-files --error-unmatch`) and unmodified
#     against HEAD in the index and the worktree (`git diff --quiet HEAD -- <file>`).
#  3. The file is ASCII, and every line (a final newline is allowed, nothing else is skipped: no blank line, no comment)
#     is exactly `name==version`: no URL, no `-e`, no `-r`, no `--index-url` or any other option, no extras, no markers.
#  4. If shared/venv/Scripts/python.exe is missing, shared/venv must not exist either; then `python -I -m venv shared/venv`
#     creates it. An existing shared/venv without python.exe refuses (something else is there).
#
# What runs: `shared/venv/Scripts/python.exe -I -m pip install --isolated --only-binary=:all: -r <copy>`. Two things are
# STRICTER than the plain brief command, never weaker: `--isolated` (pip ignores PIP_* variables and USER pip config;
# it still reads the global and site pip config, so it does NOT stop every config from adding an index: that is limited
# by the `name==version` rule and --only-binary=:all:, not by --isolated), and <copy> is a byte copy of the VALIDATED file in the scratch folder
# (scratch/venv-install/<random>.txt), so a change to the file after the checks cannot reach pip. PIP_* variables are
# also removed from this process.
# --dry-run runs every check, then only prints the commands; it creates and installs nothing.
#
# Parameters are read from $args by hand (no param block), so PowerShell's prefix matching cannot accept an
# abbreviation or a stray argument.

$ErrorActionPreference = 'Stop'
trap { [Console]::Error.WriteLine("venv-install: REFUSED: unexpected error: $($_.Exception.Message)"); exit 2 }

function Refuse([string]$why) {
    [Console]::Error.WriteLine("venv-install: REFUSED: $why")
    exit 2
}

$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$sep = [IO.Path]::DirectorySeparatorChar

# The one link check: every path this script reads, writes or runs goes through it. It refuses if any EXISTING
# component from the root down to <path> (the root itself excluded) is a junction or symlink (a reparse point), or if
# <path> is not under the root. Components that do not exist yet are skipped; they are created by this script.
function AssertNoLinks([string]$path) {
    $p = [IO.Path]::GetFullPath($path).TrimEnd($sep)
    if (-not $p.StartsWith($root.TrimEnd($sep) + $sep, [StringComparison]::OrdinalIgnoreCase)) { Refuse "'$p' is outside the root" }
    while ($p.Length -gt $root.TrimEnd($sep).Length) {
        if (Test-Path -LiteralPath $p) {
            if ((Get-Item -LiteralPath $p -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { Refuse "'$p' is a link (junction or symlink); links are not followed" }
        }
        $p = [IO.Path]::GetDirectoryName($p)
    }
}

# 1. Arguments.
$all = @($args | ForEach-Object { [string]$_ })
$dry = $false
if ($all.Count -eq 2 -and $all[1] -ceq '--dry-run') { $dry = $true; $all = @($all[0]) }
if ($all.Count -ne 1) { Refuse 'usage: venv-install.ps1 <shared/requirements-*.txt> [--dry-run] (exactly one file)' }
$req = $all[0]
if ([string]::IsNullOrWhiteSpace($req)) { Refuse 'the requirements file argument is empty' }
if ($req -match '[\x00-\x1f"*?<>|;&$`''%]') { Refuse "'$req' has a character that is not allowed in the path" }
if ($req -match '~') { Refuse "'$req' contains '~' (8.3 short names are not allowed)" }
if ([IO.Path]::IsPathRooted($req)) { Refuse "'$req' must be a path relative to the root" }
$relIn = $req.Replace('\', '/')
if ($relIn -cnotmatch '^shared/requirements-[A-Za-z0-9._-]+\.txt$' -or $relIn.Contains('..')) {
    Refuse "'$req' is not shared/requirements-<name>.txt"
}
$full = [IO.Path]::GetFullPath((Join-Path $root $relIn))
$sharedDir = Join-Path $root 'shared'
if (-not $full.StartsWith($sharedDir + $sep, [StringComparison]::OrdinalIgnoreCase)) { Refuse "'$full' is outside $sharedDir" }
if ([IO.Path]::GetDirectoryName($full) -ne $sharedDir) { Refuse "'$full' is not directly in $sharedDir" }
if (-not (Test-Path -LiteralPath $full -PathType Leaf)) { Refuse "'$relIn' does not exist" }
AssertNoLinks $full

# 2. Git: tracked and unmodified. Git environment variables could point git at another repository.
foreach ($v in 'GIT_DIR', 'GIT_WORK_TREE', 'GIT_INDEX_FILE', 'GIT_COMMON_DIR', 'GIT_OBJECT_DIRECTORY', 'GIT_CEILING_DIRECTORIES') {
    Remove-Item -LiteralPath "Env:$v" -ErrorAction SilentlyContinue
}
$top = (& git -C $root rev-parse --show-toplevel 2>$null)
if ($LASTEXITCODE -ne 0 -or -not $top) { Refuse "the root '$root' is not a git checkout" }
if ([IO.Path]::GetFullPath(([string]$top).Trim()).TrimEnd($sep) -ne $root.TrimEnd($sep)) { Refuse "the root '$root' is not the top of its git checkout ('$top')" }
& git -C $root ls-files --error-unmatch -- $relIn *> $null
if ($LASTEXITCODE -ne 0) { Refuse "'$relIn' is not tracked in git" }
& git -C $root diff --quiet HEAD -- $relIn *> $null
if ($LASTEXITCODE -ne 0) { Refuse "'$relIn' is modified against HEAD (commit it first)" }

# 3. Content: read the bytes once; these exact bytes are what pip gets.
$bytes = [IO.File]::ReadAllBytes($full)
if ($bytes.Length -eq 0) { Refuse "'$relIn' is empty" }
foreach ($b in $bytes) {
    if ($b -gt 126 -or ($b -lt 32 -and $b -ne 10 -and $b -ne 13)) { Refuse "'$relIn' has a byte that is not printable ASCII ($b)" }
}
$text = [Text.Encoding]::ASCII.GetString($bytes)
if ($text.EndsWith("`n")) { $text = $text.Substring(0, $text.Length - 1) }
if ($text.EndsWith("`r")) { $text = $text.Substring(0, $text.Length - 1) }
$n = 0
foreach ($raw in $text.Split("`n")) {
    $n++
    $line = $raw
    if ($line.EndsWith("`r")) { $line = $line.Substring(0, $line.Length - 1) }
    if ($line -cnotmatch '^[A-Za-z0-9](?:[A-Za-z0-9._-]*[A-Za-z0-9])?==[0-9][A-Za-z0-9.!+]*$') {
        Refuse "'$relIn' line ${n} '$line' is not exactly name==version"
    }
}

# 4. The venv.
$venv = Join-Path $root 'shared/venv'
$py = Join-Path $venv 'Scripts/python.exe'
$create = $false
if (-not (Test-Path -LiteralPath $py -PathType Leaf)) {
    if (Test-Path -LiteralPath $venv) { Refuse "'$venv' exists but has no Scripts/python.exe; not touching it" }
    $create = $true
}
AssertNoLinks $py

# Only now can anything run.
foreach ($v in @(Get-ChildItem Env: | Where-Object { $_.Name -like 'PIP_*' } | ForEach-Object { $_.Name })) {
    Remove-Item -LiteralPath "Env:$v" -ErrorAction SilentlyContinue
}
$copyDir = Join-Path $root 'scratch/venv-install'
$copy = Join-Path $copyDir ([Guid]::NewGuid().ToString('N') + '.txt')
AssertNoLinks $copy
if ($create) { Write-Output "venv-install: python -I -m venv $venv" }
Write-Output "venv-install: $py -I -m pip install --isolated --only-binary=:all: -r $copy (validated copy of $relIn)"
if ($dry) { Write-Output 'venv-install: dry run, nothing executed'; exit 0 }

Set-Location -LiteralPath $root
if ($create) {
    & python -I -m venv $venv
    if ($LASTEXITCODE -ne 0) { [Console]::Error.WriteLine("venv-install: creating the venv failed ($LASTEXITCODE)"); exit $LASTEXITCODE }
    if (-not (Test-Path -LiteralPath $py -PathType Leaf)) { [Console]::Error.WriteLine("venv-install: $py missing after creating the venv"); exit 1 }
    AssertNoLinks $py
}
[void](New-Item -ItemType Directory -Force -Path $copyDir)
AssertNoLinks $copy
[IO.File]::WriteAllBytes($copy, $bytes)
& $py -I -m pip install --isolated --only-binary=:all: -r $copy
exit $LASTEXITCODE
