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
#  2. The root is a git checkout (its top level). HEAD is resolved ONCE to a commit id <rev>. The file is tracked
#     (`git ls-files --error-unmatch`), unmodified against <rev> in the index and the worktree (`git diff --quiet <rev>`),
#     and its working bytes equal the RAW committed blob. Filter attributes and diff/text drivers refuse BEFORE
#     content reads or diff; no checkout conversion runs. An LF blob with CRLF working bytes refuses, even with
#     core.autocrlf=true; use byte-identical checkouts (for example, -text). skip-worktree cannot hide a change.
#  3. The bytes pip gets are ONLY the committed blob `git cat-file blob <rev>:<file>` (raw bytes from git), never the
#     working file. They are ASCII, and every line (a final newline is allowed, nothing else is skipped: no blank line,
#     no comment) is exactly `name==version`: no URL, no `-e`, no `-r`, no `--index-url` or any other option, no extras,
#     no markers. Their SHA-256 is taken once.
#  4. If shared/venv/Scripts/python.exe is missing, shared/venv must not exist either; then `python -I -m venv shared/venv`
#     creates it. An existing shared/venv without python.exe refuses (something else is there).
#
# What runs: the venv's own pip, `shared/venv/Scripts/python.exe -I -m pip install --isolated --only-binary=:all:
# --disable-pip-version-check --report <report> -r <copy>`, so the install goes into the venv itself and no --target or
# --prefix can come from config. STRICTER than the plain brief command, never weaker:
#  - No pip config is read at all: every PIP_* variable is removed and PIP_CONFIG_FILE is set to os.devnull ("nul").
#    pip 26.2.2 (pip/_internal/configuration.py, _load_config_files) skips EVERY config file (global, user, site, env)
#    when PIP_CONFIG_FILE is os.devnull; it reads that variable from os.environ directly, so it works with --isolated
#    (--isolated alone skips only the PIP_* variables and the user file; global and site config are still read).
#  - <copy> is the blob bytes written to scratch/venv-install/<random>.txt, then reopened with FileShare.Read (nobody can
#    write, rename or delete it while open), hashed again through that handle right before pip runs, and held open
#    until pip and the post-check exit; a hash mismatch refuses.
#  - --disable-pip-version-check: pip does not contact the index for its own version.
# After pip exits 0, ONE post-check (the venv's python, scratch/venv-install/<random>-check.py) is the rule "everything
# installed is in shared/venv": every distribution in pip's report and every requirement line must be installed in the
# venv at that version, and every file of each one must resolve under shared/venv. Otherwise exit 3 with the reasons.
# Known limit: this cannot detect an outside copy of a name+version also present inside; disabling pip config prevents
# config-directed installs outside the venv in the first place.
# --dry-run runs the pre-install guards, then only prints the commands; it creates and installs nothing.
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

# 2. Git: tracked and unmodified. Discard caller Git routing/config/trace variables, including variables that
# redirect reads or write trace files. Disable optional index refresh writes on these read-only checks.
foreach ($v in @(Get-ChildItem Env: | Where-Object { $_.Name -like 'GIT_*' } | ForEach-Object { $_.Name })) {
    Remove-Item -LiteralPath "Env:$v"
}
$env:GIT_OPTIONAL_LOCKS = '0'
# A missing object in a partial clone must fail, not fetch through configured remote/SSH commands.
$env:GIT_NO_LAZY_FETCH = '1'
$top = (& git --no-pager -C $root rev-parse --show-toplevel 2>$null)
if ($LASTEXITCODE -ne 0 -or -not $top) { Refuse "the root '$root' is not a git checkout" }
if ([IO.Path]::GetFullPath(([string]$top).Trim()).TrimEnd($sep) -ne $root.TrimEnd($sep)) { Refuse "the root '$root' is not the top of its git checkout ('$top')" }
$rev = ([string](& git --no-pager -C $root rev-parse --verify --quiet 'HEAD^{commit}' 2>$null)).Trim()
if ($LASTEXITCODE -ne 0 -or $rev -cnotmatch '^[0-9a-f]{40}([0-9a-f]{24})?$') { Refuse "HEAD of '$root' is not a commit" }
# Audit: rev-parse, config, check-attr and raw cat-file do not run hooks or filter drivers. Index readers and
# diff CAN invoke core.fsmonitor; refuse a configured helper before either. No config overrides (-c) are used.
& git --no-pager -C $root config --get-all core.fsmonitor *> $null
if ($LASTEXITCODE -eq 0) { Refuse 'core.fsmonitor is configured; index reads could run a helper' }
if ($LASTEXITCODE -ne 1) { Refuse 'cannot inspect core.fsmonitor configuration' }

# Check attributes before ANY content read (including diff's clean conversion). --all omits unspecified attributes,
# so an explicit filter=unspecified cannot impersonate the absence of a filter. Likewise diff=set/diff=unset can
# name drivers: conservatively refuse ALL explicit diff attributes. -text and normal text policies remain allowed.
$answer = @(& git --no-pager -C $root check-attr --all -- $relIn 2>$null)
if ($LASTEXITCODE -ne 0) { Refuse "cannot inspect attributes for '$relIn'" }
$attributes = @{}
$prefix = "${relIn}: "
foreach ($row in $answer) {
    if (-not ([string]$row).StartsWith($prefix, [StringComparison]::Ordinal) -or $row -match '[\x00-\x1f\x7f]') {
        Refuse "malformed attribute response for '$relIn'"
    }
    $parts = ([string]$row).Substring($prefix.Length) -split ': ', 2
    if ($parts.Count -ne 2 -or $parts[0] -cnotmatch '^[A-Za-z0-9_.-]+$' -or $attributes.ContainsKey($parts[0])) {
        Refuse "malformed or duplicate attribute response for '$relIn'"
    }
    $attributes[$parts[0]] = $parts[1]
}
foreach ($attr in 'filter', 'diff', 'text') {
    if (-not $attributes.ContainsKey($attr)) { continue }
    $value = $attributes[$attr]
    if ($attr -cne 'text' -or @('set', 'unset', 'auto') -cnotcontains $value) {
        Refuse "'$relIn' has unsafe '$attr' attribute '$value'; filters and drivers are not allowed"
    }
}
& git --no-pager -C $root ls-files --error-unmatch -- $relIn *> $null
if ($LASTEXITCODE -ne 0) { Refuse "'$relIn' is not tracked in git" }
# --quiet alone can still run a trusted external diff. Explicitly disable external diff AND textconv.
& git --no-pager -C $root diff --no-ext-diff --no-textconv --quiet $rev -- $relIn *> $null
if ($LASTEXITCODE -ne 0) { Refuse "'$relIn' is modified against HEAD $rev (commit it first)" }

# The committed bytes, as raw bytes from git's stdout (PowerShell's native-command pipe would decode them as text).
function GitBlob([string]$spec) {
    $psi = New-Object Diagnostics.ProcessStartInfo('git', ('--no-pager -C "{0}" cat-file blob {1}' -f $root, $spec))
    $psi.UseShellExecute = $false; $psi.RedirectStandardOutput = $true; $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    $proc = [Diagnostics.Process]::Start($psi)
    $errTask = $proc.StandardError.ReadToEndAsync()
    $ms = New-Object IO.MemoryStream
    $proc.StandardOutput.BaseStream.CopyTo($ms)
    $proc.WaitForExit()
    [void]$errTask.Result
    if ($proc.ExitCode -ne 0) { Refuse "git cat-file blob $spec failed ($($proc.ExitCode))" }
    return , $ms.ToArray()
}
function Sha256Hex([byte[]]$b) {
    $h = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($h.ComputeHash($b))).Replace('-', '').ToLowerInvariant() } finally { $h.Dispose() }
}

# 3. Content: ONLY the committed blob; these exact bytes are what pip gets.
$bytes = GitBlob "${rev}:$relIn"
$hash = Sha256Hex $bytes
# No filters, textconv or EOL conversions: only the raw stored blob is authoritative.
$working = [IO.File]::ReadAllBytes($full)
$wh = Sha256Hex $working
if ($wh -ne $hash) {
    Refuse "'$relIn' in the worktree differs from its committed bytes at $rev (modified; skip-worktree or a swap)"
}
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

# The one post-check, run by the venv's python after pip: argv = venv folder, pip report, requirements copy.
$postCheck = @'
import importlib.metadata as md, json, os, re, sys
from pip._vendor.packaging.version import Version
venv = os.path.normcase(os.path.realpath(sys.argv[1]))
def under(p):
    p = os.path.normcase(os.path.realpath(str(p)))
    return p == venv or p.startswith(venv + os.sep)
def canon(n):
    return re.sub(r"[-_.]+", "-", n).lower()
with open(sys.argv[2], encoding="utf-8") as f:
    report = json.load(f)
with open(sys.argv[3], encoding="ascii") as f:
    reqs = f.read().split()
want = [(canon(i["metadata"]["name"]), i["metadata"]["version"]) for i in report.get("install", [])]
want += [(canon(r.split("==")[0]), r.split("==")[1]) for r in reqs]
found = {}
for d in md.distributions():
    found.setdefault(canon(d.metadata["Name"]), d)
bad = []
for name, ver in want:
    d = found.get(name)
    if d is None:
        bad.append(f"{name} is not installed in the venv")
        continue
    if Version(d.version) != Version(ver):
        bad.append(f"{name} in the venv is {d.version}, not {ver}")
        continue
    if not under(d.locate_file("")):
        bad.append(f"{name} is installed at {d.locate_file('')}, outside the venv")
    for rel in d.files or []:
        if not under(d.locate_file(rel)):
            bad.append(f"{name}: {d.locate_file(rel)} is outside the venv")
if bad:
    print("\n".join(bad))
    sys.exit(1)
print(f"post-check ok: {len(want)} distribution(s) installed in the venv, every file under it")
'@

# Only now can installation run. The child environment: no PIP_* variable, and PIP_CONFIG_FILE = os.devnull so pip reads
# no config file at all (see the header).
foreach ($v in @(Get-ChildItem Env: | Where-Object { $_.Name -like 'PIP_*' } | ForEach-Object { $_.Name })) {
    Remove-Item -LiteralPath "Env:$v"
}
$env:PIP_CONFIG_FILE = 'nul'
$copyDir = Join-Path $root 'scratch/venv-install'
$stem = [Guid]::NewGuid().ToString('N')
$copy = Join-Path $copyDir "$stem.txt"
$report = Join-Path $copyDir "$stem-report.json"
$checkPy = Join-Path $copyDir "$stem-check.py"
foreach ($p in $copy, $report, $checkPy) { AssertNoLinks $p }
if ($create) { Write-Output "venv-install: python -I -m venv $venv" }
Write-Output "venv-install: $relIn at $rev, sha256 $hash"
Write-Output "venv-install: PIP_CONFIG_FILE=nul $py -I -m pip install --isolated --only-binary=:all: --disable-pip-version-check --report $report -r $copy (the committed bytes)"
if ($dry) { Write-Output 'venv-install: dry run, nothing executed'; exit 0 }

Set-Location -LiteralPath $root
if ($create) {
    & python -I -m venv $venv
    if ($LASTEXITCODE -ne 0) { [Console]::Error.WriteLine("venv-install: creating the venv failed ($LASTEXITCODE)"); exit $LASTEXITCODE }
    if (-not (Test-Path -LiteralPath $py -PathType Leaf)) { [Console]::Error.WriteLine("venv-install: $py missing after creating the venv"); exit 1 }
    AssertNoLinks $py
}
[void](New-Item -ItemType Directory -Force -Path $copyDir)
foreach ($p in $copy, $report, $checkPy) { AssertNoLinks $p }
[IO.File]::WriteAllBytes($copy, $bytes)
[IO.File]::WriteAllText($checkPy, $postCheck, (New-Object Text.UTF8Encoding($false)))
# Held open read-only, sharing only reads, until pip and the post-check are done: no one can change, rename or
# delete them meanwhile. The hash is taken through the held handle, right before pip runs.
$held = @()
try {
    foreach ($p in $copy, $checkPy) { $held += New-Object IO.FileStream($p, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read) }
    $sha = [Security.Cryptography.SHA256]::Create()
    $again = ([BitConverter]::ToString($sha.ComputeHash($held[0]))).Replace('-', '').ToLowerInvariant()
    $sha.Dispose()
    if ($again -ne $hash) { Refuse "the copy '$copy' does not hash to the committed bytes ($again, not $hash)" }
    & $py -I -m pip install --isolated --only-binary=:all: --disable-pip-version-check --report $report -r $copy
    if ($LASTEXITCODE -ne 0) { [Console]::Error.WriteLine("venv-install: pip failed ($LASTEXITCODE)"); exit $LASTEXITCODE }
    AssertNoLinks $report
    & $py -I $checkPy $venv $report $copy
    $code = $LASTEXITCODE
    if ($code -ne 0) { [Console]::Error.WriteLine("venv-install: POST-CHECK FAILED: something pip installed is not in $venv (see above)"); exit 3 }
} finally {
    foreach ($s in $held) { $s.Dispose() }
}
exit 0
