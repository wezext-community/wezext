# ============================================================
#  wezext 安装脚本（两种模式）
#
#  用法（推荐，一行装好）：
#    powershell -c "irm https://raw.githubusercontent.com/wezext-community/wezext/main/install.ps1 | iex"
#
#  二选一的安装方式：
#    1) 二进制安装（默认）—— 下载 GitHub Release 里的 zip，校验 sha256 后解压
#    2) 本地工具链编译安装 —— git clone 后 cargo build --release
#
#  因为 `irm | iex` 传不了参数，所以用环境变量控制：
#    WEZEXT_FROM_SOURCE=1   改用源码编译安装（需要 git + cargo）
#    WEZEXT_VERSION=x.y.z   指定版本（源码模式不填则用 main 分支）
#    WEZEXT_DIR=<path>      安装目录（默认 %LOCALAPPDATA%\wezext\bin）
#    WEZEXT_NO_PATH=1       不修改 PATH
#
#  也可以当文件跑：  pwsh -File install.ps1 -Source
#
#  输出只用 Write-Host + ASCII 符号，不依赖任何第三方项目/字体。
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

function Write-Step($msg) { Write-Host "==> $msg" -ForegroundColor Cyan }
function Write-Ok($msg) { Write-Host "  [ok] $msg" -ForegroundColor Green }
function Write-Warn2($msg) { Write-Host "  [!!] $msg" -ForegroundColor Yellow }

# 网络抖动时重试几次（GitHub release 资产偶尔会连不上）
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
                Write-Host "  [!!] 第 $i 次失败，3 秒后重试：$($_.Exception.Message)" -ForegroundColor Yellow
                Start-Sleep -Seconds 3
            }
        }
    }
    throw "下载失败（重试 3 次）：$url`n$($lastErr.Exception.Message)"
}

# PowerShell 5.1 某些环境没有 Get-FileHash，用 .NET 兜底
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
                Write-Host "  [!!] 查询失败，3 秒后重试：$($_.Exception.Message)" -ForegroundColor Yellow
                Start-Sleep -Seconds 3
            } else { throw }
        }
    }
}

New-Item -ItemType Directory -Force -Path $dir | Out-Null
$exe = Join-Path $dir 'wezext.exe'

if ($fromSource) {
    # ================= 方式一：源码编译安装 =================
    Write-Step '安装方式：本地工具链编译'
    foreach ($tool in 'git', 'cargo') {
        if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
            throw "源码安装需要 $tool，但没找到。装好后再试，或去掉 WEZEXT_FROM_SOURCE 用二进制安装。"
        }
    }

    $ref = if ($requested) { "v$requested" } else { 'main' }
    Write-Step "获取源码（$ref）"
    $src = Join-Path $env:TEMP 'wezext-src'
    if (Test-Path $src) { Remove-Item $src -Recurse -Force }
    & git clone --depth 1 --branch $ref "https://github.com/$repo.git" $src
    if ($LASTEXITCODE -ne 0 -and $requested) {
        Write-Warn2 "分支 $ref 不存在，回退到 main"
        Remove-Item $src -Recurse -Force -ErrorAction SilentlyContinue
        & git clone --depth 1 "https://github.com/$repo.git" $src
    }
    if ($LASTEXITCODE -ne 0) { throw 'git clone 失败' }
    Write-Ok "源码就位：$src"

    Write-Step '编译（cargo build --release）'
    Push-Location $src
    try {
        & cargo build --release --locked
        if ($LASTEXITCODE -ne 0) { throw 'cargo build 失败' }
    } finally {
        Pop-Location
    }
    $built = Join-Path $src 'target\release\wezext.exe'
    if (-not (Test-Path $built)) { throw "编译完成但没找到 $built" }
    Copy-Item $built $exe -Force
    Write-Ok "已安装到 $exe"
    if ($env:WEZEXT_KEEP_SRC -notin @('1', 'true', 'yes', 'on')) {
        Remove-Item $src -Recurse -Force -ErrorAction SilentlyContinue
        Write-Ok '已清理临时源码目录（想保留设 WEZEXT_KEEP_SRC=1）'
    }
}
else {
    # ================= 方式二：二进制安装 =================
    Write-Step '安装方式：下载预编译二进制'
    $version = if ($requested) { Write-Ok "指定版本 v$requested"; $requested } else {
        Write-Step '查询最新版本'
        $v = Resolve-LatestVersion
        Write-Ok "latest = v$v"
        $v
    }

    $asset = "wezext-v$version-x86_64-pc-windows-msvc.zip"
    $url = "https://github.com/$repo/releases/download/v$version/$asset"
    $tmp = Join-Path $env:TEMP "wezext-$version.zip"

    Write-Step "下载 $asset"
    $ProgressPreference = 'SilentlyContinue'
    Invoke-Download $url $tmp
    Write-Ok ("{0:N0} KB -> {1}" -f ((Get-Item $tmp).Length / 1KB), $tmp)

    try {
        $sumText = (Invoke-Download "$url.sha256" $null).Content
        if ($sumText -is [byte[]]) { $sumText = [Text.Encoding]::UTF8.GetString($sumText) }
        $expected = ($sumText -split '\s+')[0].Trim().ToLower()
        $actual = Get-Sha256 $tmp
        if ($expected -and $expected -ne $actual) {
            throw "sha256 不匹配`n  期望 $expected`n  实际 $actual"
        }
        Write-Ok "sha256 校验通过（$actual）"
    } catch {
        if ($_.Exception.Message -like 'sha256*') { throw }
        Write-Warn2 '没有可用的 .sha256，跳过校验'
    }

    Write-Step "解压到 $dir"
    Expand-Archive -Path $tmp -DestinationPath $dir -Force
    Remove-Item $tmp -Force -ErrorAction SilentlyContinue
    if (-not (Test-Path $exe)) { throw "解压后没找到 $exe" }
    Write-Ok 'wezext.exe 已就位'
}

# ================= PATH =================
if (-not $skipPath) {
    Write-Step '检查 PATH'
    $userPath = [Environment]::GetEnvironmentVariable('PATH', 'User')
    if (-not $userPath) { $userPath = '' }
    $parts = $userPath -split ';' | Where-Object { $_ -ne '' }
    $already = $parts | Where-Object { $_.TrimEnd('\') -ieq $dir.TrimEnd('\') }
    if ($already) {
        Write-Ok 'PATH 里已有该目录'
    } else {
        [Environment]::SetEnvironmentVariable('PATH', ((@($parts) + $dir) -join ';'), 'User')
        Write-Ok "已加入用户 PATH：$dir"
        Write-Warn2 '新开的终端才生效（当前会话下面会临时加上）'
    }
    if (($env:PATH -split ';') -notcontains $dir) { $env:PATH = "$dir;$env:PATH" }
}

# ================= 收尾 =================
Write-Host ''
try {
    $ver = & $exe --version 2>$null
    Write-Host "  wezext 安装完成：$ver" -ForegroundColor Green
} catch {
    Write-Host "  wezext 安装完成：$exe" -ForegroundColor Green
}
Write-Host ''
Write-Host '  下一步：' -ForegroundColor Cyan
Write-Host '    wezext list                  # 看看索引里有哪些扩展'
Write-Host '    wezext install wezfont       # 装字体选择器'
Write-Host '    wezext install wezlauncher   # 装标签页/启动器'
Write-Host '    wezext doctor                # 自检（含主配置是否接入 .wezterm-ext.lua）'
Write-Host ''
