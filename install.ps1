# ============================================================
#  wezext installer (two modes)
#
#  Usage (recommended, one line):
#    powershell -c "irm https://raw.githubusercontent.com/wezext-community/wezext/main/install.ps1 | iex"
#
#  Two install modes, pick one:
#    1) binary (default) - download the GitHub release zip, verify sha256, unpack
#    2) from source - git clone, then cargo build --release
#
#  `irm | iex` cannot pass arguments, so use env vars:
#    WEZEXT_FROM_SOURCE=1   install from source (needs git + cargo)
#    WEZEXT_VERSION=x.y.z   pin a version (source mode defaults to main)
#    WEZEXT_DIR=<path>      install directory (default %LOCALAPPDATA%\wezext\bin)
#    WEZEXT_NO_PATH=1       do not touch PATH
#
#  Or run it as a file:  pwsh -File install.ps1 -Source
#
#  Output uses only Write-Host + ASCII markers, no third-party dependency.
# ============================================================
param([switch]$Source)

$ErrorActionPreference = 'Stop'

try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
} catch { }

$repo = 'wezext-community/wezext'
$dir = if ($env:WEZEXT_DIR) { $env:WEZEXT_DIR } else { Join-Path $env:LOCALAPPDATA 'wezext\bin' }
$skipPath = $env:WEZEXT_NO_PATH -in @('1', 'true', 'yes', 'on')
$fromSource = $Source -or ($env:WEZEXT_FROM_SOURCE -in @('1', 'true', 'yes', 'on'))
$requested = $env:WEZEXT_VERSION

# Interactive menu: only in a real console and when no env var overrides it
$wantMenu = ($env:WEZEXT_MENU -in @('1', 'true', 'yes', 'on')) -or
    ($env:WEZEXT_YES -notin @('1', 'true', 'yes', 'on') -and
     $env:WEZEXT_FROM_SOURCE -notin @('1', 'true', 'yes', 'on') -and
     -not $Source -and
     -not [Console]::IsInputRedirected -and
     $Host.Name -eq 'ConsoleHost')

function Write-Step($msg) { Write-Host "==> $msg" -ForegroundColor Cyan }
function Write-Ok($msg) { Write-Host "  [ok] $msg" -ForegroundColor Green }
function Write-Warn2($msg) { Write-Host "  [!!] $msg" -ForegroundColor Yellow }

# Retry a few times: GitHub release assets are occasionally unreachable
function Invoke-Download([string]$url, [string]$outFile) {
    $lastErr = $null
    for ($i = 1; $i -le 3; $i++) {
        try {
            if ($outFile) {
                Invoke-WebRequest -Uri $url -OutFile $outFile -UseBasicParsing -TimeoutSec 120
            } else {
                return Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 60
            }
            return
        } catch {
            $lastErr = $_
            if ($i -lt 3) {
                Write-Host "  [!!] attempt $i failed, retrying in 3s: $($_.Exception.Message)" -ForegroundColor Yellow
                Start-Sleep -Seconds 3
            }
        }
    }
    throw "download failed after 3 attempts: $url`n$($lastErr.Exception.Message)"
}

# Some PowerShell 5.1 setups lack Get-FileHash, so fall back to .NET
function Get-Sha256([string]$path) {
    if (Get-Command Get-FileHash -ErrorAction SilentlyContinue) {
        return (Get-FileHash -Path $path -Algorithm SHA256).Hash.ToLower()
    }
    $sha = [Security.Cryptography.SHA256]::Create()
    $fs = [IO.File]::OpenRead($path)
    try { $hash = $sha.ComputeHash($fs) } finally { $fs.Dispose(); $sha.Dispose() }
    return (($hash | ForEach-Object { $_.ToString('x2') }) -join '')
}

function Resolve-LatestVersion {
    for ($i = 1; $i -le 3; $i++) {
        try {
            $rel = Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/releases/latest" `
                -Headers @{ 'User-Agent' = 'wezext-installer'; 'Accept' = 'application/vnd.github+json' } `
                -TimeoutSec 60
            return "$($rel.tag_name)".TrimStart('v')
        } catch {
            if ($i -lt 3) {
                Write-Host "  [!!] lookup failed, retrying in 3s: $($_.Exception.Message)" -ForegroundColor Yellow
                Start-Sleep -Seconds 3
            } else { throw }
        }
    }
}

# ================= interactive menu =================
if ($wantMenu) {
    Clear-Host
    Write-Host '========================================' -ForegroundColor Cyan
    Write-Host ' wezext installer' -ForegroundColor Cyan
    Write-Host ' extension manager for WezTerm' -ForegroundColor DarkGray
    Write-Host '========================================' -ForegroundColor Cyan
    Write-Host ''
    Write-Host ' Install mode:' -ForegroundColor White
    Write-Host '   1) binary     download the official release, ~250 KB (recommended)'
    Write-Host '   2) from source  needs git + cargo'
    Write-Host '   3) quit'
    Write-Host ''
    $choice = Read-Host ' Choose [1]'
    if ([string]::IsNullOrWhiteSpace($choice)) { $choice = '1' }
    switch ($choice.Trim()) {
        '1' { $fromSource = $false }
        '2' { $fromSource = $true }
        '3' { Write-Host ' Cancelled.' -ForegroundColor DarkGray; return }
        default { Write-Warn2 "invalid choice '$choice', continuing with binary"; $fromSource = $false }
    }

    Write-Host ''
    Write-Host " Install directory (Enter for default): $dir" -ForegroundColor White
    $picked = Read-Host ' Path'
    if (-not [string]::IsNullOrWhiteSpace($picked)) { $dir = $picked.Trim() }

    Write-Host ''
    $pathAnswer = Read-Host ' Add the install directory to your user PATH? [Y/n]'
    $skipPath = $pathAnswer -match '^(n|no)$'

    Write-Host ''
    Write-Host ' Installing...' -ForegroundColor Cyan
}

New-Item -ItemType Directory -Force -Path $dir | Out-Null
$exe = Join-Path $dir 'wezext.exe'

if ($fromSource) {
    # ================= mode 1: build from source =================
    Write-Step 'mode: build from source'
    foreach ($tool in 'git', 'cargo') {
        if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
            throw "building from source needs $tool. Install it, or drop WEZEXT_FROM_SOURCE to use the binary."
        }
    }

    $ref = if ($requested) { "v$requested" } else { 'main' }
    Write-Step "fetching source ($ref)"
    $src = Join-Path $env:TEMP 'wezext-src'
    if (Test-Path $src) { Remove-Item $src -Recurse -Force }
    & git clone --depth 1 --branch $ref "https://github.com/$repo.git" $src
    if ($LASTEXITCODE -ne 0 -and $requested) {
        Write-Warn2 "branch/tag $ref not found, falling back to main"
        Remove-Item $src -Recurse -Force -ErrorAction SilentlyContinue
        & git clone --depth 1 "https://github.com/$repo.git" $src
    }
    if ($LASTEXITCODE -ne 0) { throw 'git clone failed' }
    Write-Ok "source ready: $src"

    Write-Step 'building (cargo build --release)'
    Push-Location $src
    try {
        & cargo build --release --locked
        if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    } finally {
        Pop-Location
    }
    $built = Join-Path $src 'target\release\wezext.exe'
    if (-not (Test-Path $built)) { throw "build finished but $built is missing" }
    Copy-Item $built $exe -Force
    Write-Ok "installed to $exe"
    if ($env:WEZEXT_KEEP_SRC -notin @('1', 'true', 'yes', 'on')) {
        Remove-Item $src -Recurse -Force -ErrorAction SilentlyContinue
        Write-Ok 'temporary source tree removed (set WEZEXT_KEEP_SRC=1 to keep it)'
    }
}
else {
    # ================= mode 2: prebuilt binary =================
    Write-Step 'mode: prebuilt binary'
    $version = if ($requested) { Write-Ok "pinned version v$requested"; $requested } else {
        Write-Step 'resolving latest release'
        $v = Resolve-LatestVersion
        Write-Ok "latest = v$v"
        $v
    }

    $asset = "wezext-v$version-x86_64-pc-windows-msvc.zip"
    $url = "https://github.com/$repo/releases/download/v$version/$asset"
    $tmp = Join-Path $env:TEMP "wezext-$version.zip"

    Write-Step "downloading $asset"
    $ProgressPreference = 'SilentlyContinue'
    Invoke-Download $url $tmp
    Write-Ok ("{0:N0} KB -> {1}" -f ((Get-Item $tmp).Length / 1KB), $tmp)

    try {
        $sumText = (Invoke-Download "$url.sha256" $null).Content
        if ($sumText -is [byte[]]) { $sumText = [Text.Encoding]::UTF8.GetString($sumText) }
        $expected = ($sumText -split '\s+')[0].Trim().ToLower()
        $actual = Get-Sha256 $tmp
        if ($expected -and $expected -ne $actual) {
            throw "sha256 mismatch`n  expected $expected`n  actual   $actual"
        }
        Write-Ok "sha256 verified ($actual)"
    } catch {
        if ($_.Exception.Message -like 'sha256*') { throw }
        Write-Warn2 'no .sha256 available, skipping verification'
    }

    Write-Step "extracting to $dir"
    Expand-Archive -Path $tmp -DestinationPath $dir -Force
    Remove-Item $tmp -Force -ErrorAction SilentlyContinue
    if (-not (Test-Path $exe)) { throw "extraction did not produce $exe" }
    Write-Ok 'wezext.exe is in place'
}

# ================= PATH =================
if (-not $skipPath) {
    Write-Step 'checking PATH'
    $userPath = [Environment]::GetEnvironmentVariable('PATH', 'User')
    if (-not $userPath) { $userPath = '' }
    $parts = $userPath -split ';' | Where-Object { $_ -ne '' }
    $already = $parts | Where-Object { $_.TrimEnd('\') -ieq $dir.TrimEnd('\') }
    if ($already) {
        Write-Ok 'already in PATH'
    } else {
        [Environment]::SetEnvironmentVariable('PATH', ((@($parts) + $dir) -join ';'), 'User')
        Write-Ok "added to user PATH: $dir"
        Write-Warn2 'takes effect in new terminals (added to this session too)'
    }
    if (($env:PATH -split ';') -notcontains $dir) { $env:PATH = "$dir;$env:PATH" }
}

# ================= wrap up =================
Write-Host ''
try {
    $ver = & $exe --version 2>$null
    Write-Host "  wezext installed: $ver" -ForegroundColor Green
} catch {
    Write-Host "  wezext installed: $exe" -ForegroundColor Green
}
Write-Host ''
if ($wantMenu) {
    $ans = Read-Host ' Install the recommended extensions (wezfont / wezlauncher / opacity-toggle)? [Y/n]'
    if ($ans -notmatch '^(n|no)$') {
        Write-Step 'installing recommended extensions'
        & $exe install wezfont wezlauncher opacity-toggle
        if ($LASTEXITCODE -eq 0) {
            Write-Ok 'extensions installed (open a new WezTerm window to activate)'
        } else {
            Write-Warn2 'some failed, run wezext install later if needed'
        }
        Write-Host ''
        Write-Step 'self-check'
        & $exe doctor
        Write-Host ''
    }
}

Write-Host ' Next steps:' -ForegroundColor Cyan
Write-Host '    wezext list                  # what is in the index'
Write-Host '    wezext install wezfont       # font picker'
Write-Host '    wezext install wezlauncher   # tabs/launcher'
Write-Host '    wezext doctor                # self-check (also verifies .wezterm-ext.lua is wired up)'
Write-Host ''
