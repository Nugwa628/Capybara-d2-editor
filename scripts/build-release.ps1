<#
.SYNOPSIS
Builds, checks and zips the release of Capybara's D2 Editor.

.DESCRIPTION
Steps (see docs/release-checklist.md, "Packaging"):

  1. THIRD-PARTY-LICENSES.md must be up to date
     (python -I scripts/third_party_licenses.py --check).
  2. cargo build --release -p capy-app --target x86_64-pc-windows-msvc with a
     static CRT and remapped source paths, into its own target folder
     (target\release-dist) so dev builds are not invalidated.
  3. Checks on the exe: PE32+ x64, Windows GUI subsystem, no VCRUNTIME140 /
     api-ms-win-crt / ucrtbase / msvcp imports (import and delay-import tables
     are parsed here, nothing is downloaded), no occurrence of the builder's
     user folder, repository, cargo or rustup paths in the bytes (ASCII and
     UTF-16, any case), and the size.
  4. dist\CapybarasD2Editor-<version>-win64.zip with one top folder holding
     CapybarasD2Editor.exe, README.md, LICENSE*, NOTICE,
     THIRD-PARTY-LICENSES.md, docs\user-guide.md and docs\cli.md.

Windows PowerShell 5.1 compatible. Game folders are never touched.

.PARAMETER SkipBuild
Do not compile; check and package the exe already in the release target
folder (or -ExePath).

.PARAMETER DryRun
Print every step and run only the read-only checks (license notice, files
to package, the exe if one exists). Builds nothing and writes nothing.

.PARAMETER ExePath
Check and package this exe instead of the build output (implies -SkipBuild).
It is renamed to CapybarasD2Editor.exe in the zip.

.PARAMETER Version
Version in the zip name. Default: [workspace.package] version in Cargo.toml.

.PARAMETER OutDir
Where the zip goes. Default: <repo>\dist (gitignored).

.EXAMPLE
powershell -ExecutionPolicy Bypass -File scripts\build-release.ps1

.EXAMPLE
powershell -ExecutionPolicy Bypass -File scripts\build-release.ps1 -SkipBuild

.EXAMPLE
powershell -ExecutionPolicy Bypass -File scripts\build-release.ps1 -DryRun
#>
[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [switch]$DryRun,
    [string]$ExePath,
    [string]$Version,
    [string]$OutDir
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

$Target = 'x86_64-pc-windows-msvc'
$ProductExe = 'CapybarasD2Editor.exe'
# The bin is being renamed; the old name is accepted and renamed in the zip.
$ExeNames = @('CapybarasD2Editor.exe', 'CapybaraMapEditor.exe')
# Imports that a clean Windows 10/11 does not have without a redistributable
# (VC++ runtime) or that mean the CRT is not static.
$ForbiddenImports = '^(vcruntime\d+.*|msvcp\d+.*|ucrtbase.*|api-ms-win-crt-.*)\.dll$'

$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path.TrimEnd('\')
$TargetDir = Join-Path $Repo 'target\release-dist'
if (-not $OutDir) { $OutDir = Join-Path $Repo 'dist' }
if ($ExePath) { $SkipBuild = $true }

$script:Failures = New-Object System.Collections.Generic.List[string]

function Step([string]$Text) { Write-Host ''; Write-Host "== $Text" -ForegroundColor Cyan }
function Info([string]$Text) { Write-Host "   $Text" }
function Ok([string]$Text) { Write-Host "   ok: $Text" -ForegroundColor Green }
function Fail([string]$Text) {
    Write-Host "   FAIL: $Text" -ForegroundColor Red
    $script:Failures.Add($Text)
}
function Stop-IfFailed {
    if ($script:Failures.Count -gt 0) {
        Write-Host ''
        Write-Host "build-release: $($script:Failures.Count) check(s) failed:" -ForegroundColor Red
        foreach ($f in $script:Failures) { Write-Host "  - $f" -ForegroundColor Red }
        exit 1
    }
}

# --- paths that must not end up in the exe --------------------------------

function Get-CargoHome {
    if ($env:CARGO_HOME) { return $env:CARGO_HOME.TrimEnd('\') }
    return (Join-Path $env:USERPROFILE '.cargo')
}

function Get-RustupHome {
    if ($env:RUSTUP_HOME) { return $env:RUSTUP_HOME.TrimEnd('\') }
    return (Join-Path $env:USERPROFILE '.rustup')
}

function Get-RustFlags {
    $cargoHome = Get-CargoHome
    $rustupHome = Get-RustupHome
    # rustc applies the last matching remap; these prefixes do not nest
    # (repository, cargo home and rustup home are separate folders).
    return @(
        '-C', 'target-feature=+crt-static',
        "--remap-path-prefix=$Repo=.",
        "--remap-path-prefix=$cargoHome=cargo",
        "--remap-path-prefix=$rustupHome=rustup"
    ) -join ' '
}

# --- version ---------------------------------------------------------------

function Get-WorkspaceVersion {
    $section = ''
    foreach ($line in Get-Content -LiteralPath (Join-Path $Repo 'Cargo.toml')) {
        $t = $line.Trim()
        if ($t -match '^\[(.+)\]$') { $section = $Matches[1]; continue }
        if ($section -eq 'workspace.package' -and $t -match '^version\s*=\s*"([^"]+)"') {
            return $Matches[1]
        }
    }
    throw 'no [workspace.package] version in Cargo.toml; pass -Version'
}

# --- PE parsing ------------------------------------------------------------

function Get-RvaOffset($Sections, [uint32]$Rva) {
    foreach ($s in $Sections) {
        $size = [Math]::Max([uint64]$s.VirtualSize, [uint64]$s.RawSize)
        if ($Rva -ge $s.Va -and [uint64]$Rva -lt ([uint64]$s.Va + $size)) {
            return [int64]$Rva - $s.Va + $s.RawPtr
        }
    }
    return -1
}

function Read-CString([byte[]]$Bytes, [int64]$Offset) {
    if ($Offset -lt 0 -or $Offset -ge $Bytes.Length) { return $null }
    $end = $Offset
    $limit = [Math]::Min($Bytes.Length, $Offset + 512)
    while ($end -lt $limit -and $Bytes[$end] -ne 0) { $end++ }
    return [Text.Encoding]::ASCII.GetString($Bytes, [int]$Offset, [int]($end - $Offset))
}

# Machine, subsystem and the DLL names of the import and delay-import tables.
function Read-PeInfo([byte[]]$Bytes) {
    if ($Bytes.Length -lt 0x40 -or $Bytes[0] -ne 0x4D -or $Bytes[1] -ne 0x5A) {
        throw 'not a PE file (no MZ header)'
    }
    $pe = [BitConverter]::ToInt32($Bytes, 0x3C)
    if ($pe -lt 0 -or $pe + 24 -gt $Bytes.Length -or
        [BitConverter]::ToUInt32($Bytes, $pe) -ne 0x00004550) {
        throw 'not a PE file (no PE signature)'
    }
    $machine = [BitConverter]::ToUInt16($Bytes, $pe + 4)
    $sectionCount = [BitConverter]::ToUInt16($Bytes, $pe + 6)
    $optSize = [BitConverter]::ToUInt16($Bytes, $pe + 20)
    $opt = $pe + 24
    $magic = [BitConverter]::ToUInt16($Bytes, $opt)
    if ($magic -eq 0x20B) {
        $imageBase = [BitConverter]::ToUInt64($Bytes, $opt + 24)
        $dirCount = [BitConverter]::ToUInt32($Bytes, $opt + 108)
        $dirs = $opt + 112
    } elseif ($magic -eq 0x10B) {
        $imageBase = [uint64][BitConverter]::ToUInt32($Bytes, $opt + 28)
        $dirCount = [BitConverter]::ToUInt32($Bytes, $opt + 92)
        $dirs = $opt + 96
    } else {
        throw ('unknown optional header magic 0x{0:X}' -f $magic)
    }
    $subsystem = [BitConverter]::ToUInt16($Bytes, $opt + 68)

    $sections = @()
    $sec = $opt + $optSize
    for ($i = 0; $i -lt $sectionCount; $i++) {
        $o = $sec + 40 * $i
        $sections += [pscustomobject]@{
            Name        = [Text.Encoding]::ASCII.GetString($Bytes, $o, 8).TrimEnd([char]0)
            VirtualSize = [BitConverter]::ToUInt32($Bytes, $o + 8)
            Va          = [BitConverter]::ToUInt32($Bytes, $o + 12)
            RawSize     = [BitConverter]::ToUInt32($Bytes, $o + 16)
            RawPtr      = [BitConverter]::ToUInt32($Bytes, $o + 20)
        }
    }

    $imports = New-Object System.Collections.Generic.List[string]
    # Directory 1: imports, 20-byte descriptors, Name RVA at +12.
    if ($dirCount -gt 1) {
        $rva = [BitConverter]::ToUInt32($Bytes, $dirs + 8)
        $off = Get-RvaOffset $sections $rva
        if ($rva -ne 0 -and $off -ge 0) {
            for ($k = 0; $k -lt 4096; $k++) {
                $d = $off + 20 * $k
                if ($d + 20 -gt $Bytes.Length) { break }
                $nameRva = [BitConverter]::ToUInt32($Bytes, $d + 12)
                $thunk = [BitConverter]::ToUInt32($Bytes, $d + 16)
                if ($nameRva -eq 0 -and $thunk -eq 0) { break }
                $n = Read-CString $Bytes (Get-RvaOffset $sections $nameRva)
                if ($n) { $imports.Add($n) }
            }
        }
    }
    # Directory 13: delay imports, 32-byte descriptors, DllName at +4 (an RVA
    # when bit 0 of the attributes is set, else a VA).
    $delay = New-Object System.Collections.Generic.List[string]
    if ($dirCount -gt 13) {
        $rva = [BitConverter]::ToUInt32($Bytes, $dirs + 13 * 8)
        $off = Get-RvaOffset $sections $rva
        if ($rva -ne 0 -and $off -ge 0) {
            for ($k = 0; $k -lt 4096; $k++) {
                $d = $off + 32 * $k
                if ($d + 32 -gt $Bytes.Length) { break }
                $attr = [BitConverter]::ToUInt32($Bytes, $d)
                $name = [uint64][BitConverter]::ToUInt32($Bytes, $d + 4)
                if ($name -eq 0) { break }
                if (($attr -band 1) -eq 0) {
                    if ($name -lt $imageBase) { continue }
                    $name = $name - $imageBase
                }
                $n = Read-CString $Bytes (Get-RvaOffset $sections ([uint32]$name))
                if ($n) { $delay.Add($n) }
            }
        }
    }
    return [pscustomobject]@{
        Machine      = $machine
        Magic        = $magic
        Subsystem    = $subsystem
        Imports      = $imports.ToArray()
        DelayImports = $delay.ToArray()
    }
}

# Occurrences of $Needle in $Haystack (a Latin-1 view of the bytes), as ASCII
# and as UTF-16LE, ignoring case.
function Count-Needle([string]$Haystack, [string]$Needle) {
    $latin1 = [Text.Encoding]::GetEncoding(28591)
    $forms = @($Needle, $latin1.GetString([Text.Encoding]::Unicode.GetBytes($Needle)))
    $count = 0
    foreach ($f in $forms) {
        $i = 0
        while (($i = $Haystack.IndexOf($f, $i, [StringComparison]::OrdinalIgnoreCase)) -ge 0) {
            $count++
            $i += $f.Length
        }
    }
    return $count
}

function Test-Exe([string]$Path) {
    Step "Checking $Path"
    $bytes = [IO.File]::ReadAllBytes($Path)
    $mib = '{0:N1}' -f ($bytes.Length / 1MB)
    Info ("size: {0:N0} bytes ({1} MiB)" -f $bytes.Length, $mib)

    try {
        $pe = Read-PeInfo $bytes
    } catch {
        Fail "cannot read the PE headers: $($_.Exception.Message)"
        return
    }
    if ($pe.Machine -eq 0x8664 -and $pe.Magic -eq 0x20B) { Ok 'PE32+ x86-64' }
    else { Fail ('machine 0x{0:X4} / magic 0x{1:X}, expected x86-64 PE32+' -f $pe.Machine, $pe.Magic) }
    if ($pe.Subsystem -eq 2) { Ok 'subsystem 2 (Windows GUI)' }
    else { Fail "subsystem $($pe.Subsystem), expected 2 (Windows GUI; 3 is a console exe)" }

    Info ("imports: " + (($pe.Imports | Sort-Object) -join ', '))
    if ($pe.DelayImports.Count -gt 0) {
        Info ("delay imports: " + (($pe.DelayImports | Sort-Object) -join ', '))
    }
    $bad = @(@($pe.Imports) + @($pe.DelayImports) | Where-Object { $_ -match $ForbiddenImports })
    if ($bad.Count -eq 0) { Ok 'no VC++ runtime or UCRT imports (static CRT)' }
    else { Fail ("imports that need the VC++ redistributable or a dynamic CRT: " + ($bad -join ', ')) }

    $latin1 = [Text.Encoding]::GetEncoding(28591)
    $text = $latin1.GetString($bytes)
    $needles = New-Object System.Collections.Generic.List[string]
    foreach ($p in @($env:USERPROFILE, $Repo, (Get-CargoHome), (Get-RustupHome))) {
        if (-not $p) { continue }
        $p = $p.TrimEnd('\')
        foreach ($form in @($p, $p.Replace('\', '/'), $p.Replace('\', '\\'))) {
            if (-not $needles.Contains($form)) { $needles.Add($form) }
        }
    }
    $leaks = 0
    foreach ($n in $needles) {
        $c = Count-Needle $text $n
        if ($c -gt 0) {
            Info "found $c x '$n'"
            $leaks += $c
        }
    }
    if ($leaks -eq 0) { Ok "no builder paths ($($needles.Count) patterns: user folder, repository, cargo, rustup)" }
    else { Fail "the exe contains the builder's paths (counts above; patterns overlap)" }
}

# --- main ------------------------------------------------------------------

if (-not $Version) { $Version = Get-WorkspaceVersion }
$PackageName = "CapybarasD2Editor-$Version-win64"
$ZipPath = Join-Path $OutDir "$PackageName.zip"
$Mode = if ($DryRun) { 'dry run' } elseif ($SkipBuild) { 'skip build' } else { 'full build' }

Write-Host "Capybara's D2 Editor release $Version ($Mode)"
Info "repository: $Repo"
Info "zip:        $ZipPath"

Push-Location $Repo
try {
    Step 'Third-party license notice'
    $python = Get-Command python -ErrorAction SilentlyContinue
    if (-not $python) {
        Fail 'python not found on PATH (needed for scripts/third_party_licenses.py --check)'
    } else {
        Info 'python -I scripts/third_party_licenses.py --check'
        & python -I scripts/third_party_licenses.py --check
        if ($LASTEXITCODE -ne 0) {
            Fail 'THIRD-PARTY-LICENSES.md is stale: run python -I scripts/third_party_licenses.py and commit it'
        } else { Ok 'THIRD-PARTY-LICENSES.md is up to date' }
    }

    Step 'Files to package'
    $files = New-Object System.Collections.Generic.List[object]
    $licenses = @(Get-ChildItem -LiteralPath $Repo -File | Where-Object { $_.Name -like 'LICENSE*' } | Sort-Object Name)
    if ($licenses.Count -eq 0) { Fail 'no LICENSE* file in the repository root' }
    foreach ($rel in @('README.md') + @($licenses | ForEach-Object { $_.Name }) +
             @('NOTICE', 'THIRD-PARTY-LICENSES.md', 'docs\user-guide.md', 'docs\cli.md')) {
        $src = Join-Path $Repo $rel
        if (Test-Path -LiteralPath $src -PathType Leaf) {
            $files.Add([pscustomobject]@{ Source = $src; Entry = $rel.Replace('\', '/') })
            Info $rel
        } else {
            Fail "missing $rel"
        }
    }
    Stop-IfFailed

    $releaseDir = Join-Path $TargetDir "$Target\release"
    $rustFlags = Get-RustFlags
    Step 'Build'
    if ($SkipBuild -or $DryRun) {
        Info "skipped; would run with RUSTFLAGS=$rustFlags"
        Info "cargo build --release -p capy-app --target $Target --target-dir $TargetDir"
    } else {
        Info "RUSTFLAGS=$rustFlags"
        Info "cargo build --release -p capy-app --target $Target --target-dir $TargetDir"
        $saved = @{ RUSTFLAGS = $env:RUSTFLAGS; CARGO_ENCODED_RUSTFLAGS = $env:CARGO_ENCODED_RUSTFLAGS }
        $started = Get-Date
        try {
            $env:RUSTFLAGS = $rustFlags
            # CARGO_ENCODED_RUSTFLAGS would win over RUSTFLAGS.
            Remove-Item Env:CARGO_ENCODED_RUSTFLAGS -ErrorAction SilentlyContinue
            & cargo build --release -p capy-app --target $Target --target-dir $TargetDir
            $code = $LASTEXITCODE
        } finally {
            $env:RUSTFLAGS = $saved.RUSTFLAGS
            if ($null -ne $saved.CARGO_ENCODED_RUSTFLAGS) { $env:CARGO_ENCODED_RUSTFLAGS = $saved.CARGO_ENCODED_RUSTFLAGS }
        }
        if ($code -ne 0) { Fail "cargo build failed (exit $code)"; Stop-IfFailed }
        Ok ('built in {0:N1} min' -f ((Get-Date) - $started).TotalMinutes)
    }

    if ($ExePath) {
        $exe = (Resolve-Path -LiteralPath $ExePath).Path
    } else {
        $exe = $null
        foreach ($n in $ExeNames) {
            $candidate = Join-Path $releaseDir $n
            if (Test-Path -LiteralPath $candidate -PathType Leaf) { $exe = $candidate; break }
        }
    }
    if (-not $exe) {
        if ($DryRun) {
            Info "no exe in $releaseDir yet; the exe checks run after a build"
        } else {
            Fail "no $($ExeNames -join ' or ') in $releaseDir (build first, or pass -ExePath)"
        }
    } else {
        Test-Exe $exe
    }
    Stop-IfFailed

    Step 'Package'
    $exeShown = if ($exe) { $exe } else { "(the build output in $releaseDir)" }
    Info "$PackageName/$ProductExe  <-  $exeShown"
    foreach ($f in $files) { Info "$PackageName/$($f.Entry)" }
    if ($DryRun) {
        Info "dry run: $ZipPath not written"
    } else {
        Add-Type -AssemblyName System.IO.Compression
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
        if (Test-Path -LiteralPath $ZipPath) { Remove-Item -LiteralPath $ZipPath -Force }
        $level = [System.IO.Compression.CompressionLevel]::Optimal
        $zip = [System.IO.Compression.ZipFile]::Open($ZipPath, [System.IO.Compression.ZipArchiveMode]::Create)
        try {
            # Forward slashes in entry names (Compress-Archive in 5.1 writes
            # backslashes, which other unzip tools mis-read).
            [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, $exe, "$PackageName/$ProductExe", $level)
            foreach ($f in $files) {
                [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, $f.Source, "$PackageName/$($f.Entry)", $level)
            }
        } finally {
            $zip.Dispose()
        }
        $check = [System.IO.Compression.ZipFile]::OpenRead($ZipPath)
        try {
            $entries = @($check.Entries | ForEach-Object { $_.FullName })
        } finally {
            $check.Dispose()
        }
        if ($entries.Count -ne $files.Count + 1) { Fail "the zip holds $($entries.Count) entries, expected $($files.Count + 1)" }
        Stop-IfFailed
        $zipSize = (Get-Item -LiteralPath $ZipPath).Length
        $hash = (Get-FileHash -LiteralPath $ZipPath -Algorithm SHA256).Hash.ToLowerInvariant()
        Ok ("{0} ({1:N0} bytes, {2} entries)" -f $ZipPath, $zipSize, $entries.Count)
        Info "sha256: $hash"
    }
} finally {
    Pop-Location
}

Write-Host ''
Write-Host "build-release: done ($Mode)" -ForegroundColor Green
exit 0
