# ============================================================
#  wezext 一键安装脚本
#
#  用法（PowerShell）：
#    irm https://raw.githubusercontent.com/wezext-community/wezext/main/install.ps1 | iex
#
#  可选环境变量（适合无参数管道执行）：
#    WEZEXT_VERSION  指定版本号，如 0.1.4（默认取 latest release）
#    WEZEXT_DIR      安装目录（默认 %LOCALAPPDATA%\wezext\bin）
#    WEZEXT_NO_PATH  设为 1 则不修改 PATH
#
#  脚本做的事：找最新 release → 下载 zip → 校验 sha256 → 解压 →
#             加进用户 PATH → 打印版本
# ============================================================
$ErrorActionPreference = 'Stop'

try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
} catch {
    # 新版 PowerShell 不需要，忽略
}

$repo = 'wezext-community/wezext'
$dir = if ($env:WEZEXT_DIR) { $env:WEZEXT_DIR } else { Join-Path $env:LOCALAPPDATA 'wezext\bin' }
$skipPath = $env:WEZEXT_NO_PATH -in @('1', 'true', 'yes', 'on')

# PowerShell 5.1 某些环境里没有 Get-FileHash，用 .NET 兜底
function Get-Sha256([string]$path) {
    $cmd = Get-Command Get-FileHash -ErrorAction SilentlyContinue
    if ($cmd) { return (Get-FileHash -Path $path -Algorithm SHA256).Hash.ToLower() }
    $sha = [Security.Cryptography.SHA256]::Create()
    $fs = [IO.File]::OpenRead($path)
    try { $hash = $sha.ComputeHash($fs) } finally { $fs.Dispose(); $sha.Dispose() }
    return (($hash | ForEach-Object { $_.ToString('x2') }) -join '')
}

function Write-Step($msg) { Write-Host "==> $msg" -ForegroundColor Cyan }
function Write-Ok($msg) { Write-Host "  ✓ $msg" -ForegroundColor Green }
function Write-Warn2($msg) { Write-Host "  ! $msg" -ForegroundColor Yellow }

# ---------- 1. 确定版本 ----------
$version = $env:WEZEXT_VERSION
if (-not $version) {
    Write-Step '查询最新版本'
    $rel = Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/releases/latest" `
        -Headers @{ 'User-Agent' = 'wezext-installer'; 'Accept' = 'application/vnd.github+json' }
    $version = "$($rel.tag_name)".TrimStart('v')
    Write-Ok "latest = v$version"
} else {
    Write-Step "使用指定版本 v$version"
}

$asset = "wezext-v$version-x86_64-pc-windows-msvc.zip"
$url = "https://github.com/$repo/releases/download/v$version/$asset"

# ---------- 2. 下载 ----------
$tmp = Join-Path $env:TEMP "wezext-$version.zip"
Write-Step "下载 $asset"
$ProgressPreference = 'SilentlyContinue' # PS5.1 下加速
Invoke-WebRequest -Uri $url -OutFile $tmp -UseBasicParsing
Write-Ok ("{0:N0} KB -> {1}" -f ((Get-Item $tmp).Length / 1KB), $tmp)

# ---------- 3. 校验 sha256（有就校验） ----------
try {
    $sumText = (Invoke-WebRequest -Uri "$url.sha256" -UseBasicParsing).Content
    if ($sumText -is [byte[]]) { $sumText = [Text.Encoding]::UTF8.GetString($sumText) }
    $expected = ($sumText -split '\s+')[0].Trim().ToLower()
    $actual = Get-Sha256 $tmp
    if ($expected -and $expected -ne $actual) {
        throw "sha256 不匹配`n  期望 $expected`n  实际 $actual"
    }
    Write-Ok "sha256 校验通过（$actual）"
} catch [System.Net.WebException] {
    Write-Warn2 '发布里没有 .sha256，跳过校验'
} catch {
    if ($_.Exception.Message -like 'sha256*') { throw }
    Write-Warn2 "跳过 sha256 校验：$($_.Exception.Message)"
}

# ---------- 4. 解压 ----------
Write-Step "安装到 $dir"
New-Item -ItemType Directory -Force -Path $dir | Out-Null
Expand-Archive -Path $tmp -DestinationPath $dir -Force
Remove-Item $tmp -Force -ErrorAction SilentlyContinue
$exe = Join-Path $dir 'wezext.exe'
if (-not (Test-Path $exe)) { throw "解压后没找到 $exe" }
Write-Ok "wezext.exe 已就位"

# ---------- 5. 加进用户 PATH ----------
if (-not $skipPath) {
    Write-Step '检查 PATH'
    $userPath = [Environment]::GetEnvironmentVariable('PATH', 'User')
    if (-not $userPath) { $userPath = '' }
    $parts = $userPath -split ';' | Where-Object { $_ -ne '' }
    $already = $parts | Where-Object { $_.TrimEnd('\') -ieq $dir.TrimEnd('\') }
    if ($already) {
        Write-Ok 'PATH 里已有该目录'
    } else {
        $newPath = (@($parts) + $dir) -join ';'
        [Environment]::SetEnvironmentVariable('PATH', $newPath, 'User')
        Write-Ok "已加入用户 PATH：$dir"
        Write-Warn2 '新开的终端才会生效（当前会话下面会临时加上）'
    }
    if (($env:PATH -split ';') -notcontains $dir) { $env:PATH = "$dir;$env:PATH" }
}

# ---------- 6. 收尾 ----------
try {
    $ver = & $exe --version 2>$null
    Write-Host ''
    Write-Host "  wezext 安装完成：$ver" -ForegroundColor Green
} catch {
    Write-Host ''
    Write-Host "  wezext 安装完成：$exe" -ForegroundColor Green
}
Write-Host ''
Write-Host '  下一步：' -ForegroundColor Cyan
Write-Host '    wezext list                  # 看看索引里有哪些扩展'
Write-Host '    wezext install wezfont       # 装字体选择器'
Write-Host '    wezext install wezlauncher   # 装标签页/启动器'
Write-Host '    wezext doctor                # 自检（含主配置是否接入 .wezterm-ext.lua）'
Write-Host ''
