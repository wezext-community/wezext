// ============================================================
//  wezext —— wezterm 扩展管理器
//
//  索引：wezext-index 仓库（index.json），可用 WEZEXT_INDEX 覆盖为本地目录/文件/URL
//  安装：%LOCALAPPDATA%\wezext\{bin,lua,src}\<name>\
//  配置：生成 ~/.wezterm-ext.lua，由 .wezterm.lua 末尾 dofile 进来
//        （wezext 永不改你的主配置）
//
//  用法： wezext <command>
//    list                 列出索引里的扩展和安装状态
//    install <name>...    安装（默认二进制；--build 走源码构建；--lua 只装 Lua 片段）
//    remove <name>...     卸载
//    update [name]...     更新（不写名字就全部）
//    build <name>...      从源码构建（需要 Rust 工具链）
//    doctor               自检：状态、文件、配置是否已接入
//    index                显示索引来源，并刷新缓存
// ============================================================
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ---------------- 数据结构 ----------------

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Index {
    #[serde(default)]
    schema: u32,
    #[serde(default)]
    extensions: BTreeMap<String, Extension>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Extension {
    name: String,
    #[serde(default)]
    display: String,
    #[serde(default)]
    description: String,
    version: String,
    #[serde(default)]
    homepage: String,
    #[serde(default)]
    source: Option<SourceSpec>,
    #[serde(default)]
    binary: Option<BinarySpec>,
    #[serde(default)]
    lua: Option<String>,
    #[serde(default)]
    keys: Vec<KeySpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SourceSpec {
    repo: String,
    #[serde(default)]
    crate_: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct BinarySpec {
    /// 发布的资产名模板，支持 {version}
    asset: String,
    /// 压缩包里可执行文件名
    exe: String,
    /// 下载地址模板，支持 {version} / {asset}
    url: String,
    /// 可选的 sha256（小写十六进制）
    #[serde(default)]
    sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct KeySpec {
    key: String,
    #[serde(default = "default_mods")]
    mods: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    hint: String,
}

fn default_mods() -> String {
    "NONE".to_string()
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
struct State {
    #[serde(default)]
    extensions: BTreeMap<String, Installed>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Installed {
    version: String,
    /// "binary" | "source" | "lua"
    kind: String,
    /// 安装目录（lua 扩展是那个文件路径）
    path: String,
    #[serde(default)]
    exe: String,
}

// ---------------- 路径 ----------------

fn home() -> PathBuf {
    PathBuf::from(std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into()))
}

fn local_appdata() -> PathBuf {
    std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home().join("AppData/Local"))
}

fn root_dir() -> PathBuf {
    local_appdata().join("wezext")
}
fn bin_dir() -> PathBuf {
    root_dir().join("bin")
}
fn lua_dir() -> PathBuf {
    root_dir().join("lua")
}
fn src_dir() -> PathBuf {
    root_dir().join("src")
}
fn cache_dir() -> PathBuf {
    root_dir().join("cache")
}
fn state_path() -> PathBuf {
    root_dir().join("state.json")
}
fn ext_lua_path() -> PathBuf {
    home().join(".wezterm-ext.lua")
}
fn main_config_path() -> PathBuf {
    home().join(".wezterm.lua")
}

const INCLUDE_HINT: &str = "pcall(dofile, wezterm.home_dir .. '/.wezterm-ext.lua')";

// ---------------- 索引 ----------------

fn index_source() -> String {
    std::env::var("WEZEXT_INDEX").unwrap_or_else(|_| {
        "https://raw.githubusercontent.com/wezext-community/wezext-index/main/index.json".to_string()
    })
}

fn load_index(refresh: bool) -> Result<(Index, String), String> {
    let src = index_source();

    // 本地目录
    let as_path = PathBuf::from(&src);
    if as_path.is_dir() {
        let file = as_path.join("index.json");
        let text = fs::read_to_string(&file)
            .map_err(|e| format!("读取 {} 失败: {e}", file.display()))?;
        let index: Index =
            serde_json::from_str(&text).map_err(|e| format!("解析 {} 失败: {e}", file.display()))?;
        return Ok((index, src));
    }
    // 本地文件
    if as_path.is_file() {
        let text = fs::read_to_string(&as_path)
            .map_err(|e| format!("读取 {} 失败: {e}", as_path.display()))?;
        let index: Index = serde_json::from_str(&text)
            .map_err(|e| format!("解析 {} 失败: {e}", as_path.display()))?;
        return Ok((index, src));
    }

    // 远程：可以缓存
    let cache = cache_dir().join("index.json");
    if !refresh && cache.is_file() {
        if let Ok(text) = fs::read_to_string(&cache) {
            if let Ok(index) = serde_json::from_str::<Index>(&text) {
                return Ok((index, format!("{src}（缓存）")));
            }
        }
    }
    fs::create_dir_all(cache_dir()).map_err(|e| format!("创建缓存目录失败: {e}"))?;
    curl_to(&src, &cache)?;
    let text = fs::read_to_string(&cache).map_err(|e| format!("读取缓存失败: {e}"))?;
    let index: Index = serde_json::from_str(&text).map_err(|e| format!("解析索引失败: {e}"))?;
    Ok((index, src))
}

// ---------------- 外部工具 ----------------

fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd.output().map_err(|e| format!("启动进程失败: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "命令失败（{}）: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// 用 Windows 自带的 curl.exe 下载
fn curl_to(url: &str, dest: &Path) -> Result<(), String> {
    if dest.exists() {
        let _ = fs::remove_file(dest);
    }
    run(Command::new("curl.exe").args([
        "-fsSL",
        "--retry",
        "2",
        "-o",
        &dest.to_string_lossy(),
        url,
    ]))
    .map_err(|e| format!("下载 {url} 失败: {e}"))
}

/// 用 Windows 自带的 tar.exe 解压 zip
fn extract_zip(zip: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| format!("创建 {} 失败: {e}", dest.display()))?;

    // 首选 bsdtar；某些工具产出的 zip 它读不了（比如 PowerShell 的 Compress-Archive），
    // 这时退回 PowerShell 的 Expand-Archive。
    if run(Command::new("tar.exe").args([
        "-xf",
        &zip.to_string_lossy(),
        "-C",
        &dest.to_string_lossy(),
    ]))
    .is_ok()
    {
        return Ok(());
    }

    let script = format!(
        "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
        zip.display(),
        dest.display()
    );
    run(Command::new("powershell.exe").args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &script,
    ]))
    .map_err(|e| format!("解压失败（tar 和 Expand-Archive 都失败）: {e}"))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let data = fs::read(path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>())
}

// ---------------- 状态 ----------------

fn load_state() -> State {
    fs::read_to_string(state_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_state(state: &State) -> Result<(), String> {
    fs::create_dir_all(root_dir()).map_err(|e| format!("创建 {} 失败: {e}", root_dir().display()))?;
    let text = serde_json::to_string_pretty(state).map_err(|e| format!("序列化失败: {e}"))?;
    atomic_write(&state_path(), &text)
}

/// 先写临时文件再改名，避免 wezterm 之类读到半截文件
fn atomic_write(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text).map_err(|e| format!("写入 {} 失败: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("替换 {} 失败: {e}", path.display()))
}

// ---------------- 生成 ~/.wezterm-ext.lua ----------------

fn lua_quote(s: &str) -> String {
    // Lua 长字符串最省事，且不会踩转义坑
    format!("[[{}]]", s)
}

fn regenerate_lua(state: &State, index: &Index) -> Result<PathBuf, String> {
    let mut body = String::new();
    body.push_str("-- 由 wezext 自动生成，请勿手改（改完 wezext 会覆盖）\n");
    body.push_str("-- 主配置末尾应有：");
    body.push_str(INCLUDE_HINT);
    body.push_str("\n\nlocal wezterm = require 'wezterm'\n\nlocal keys = {}\nlocal apply = {}\n\n");

    for (name, inst) in &state.extensions {
        let Some(ext) = index.extensions.get(name) else {
            continue;
        };
        body.push_str(&format!("-- {} {}\n", name, inst.version));

        // 键位：调用安装好的可执行文件
        if inst.kind != "lua" && !inst.exe.is_empty() {
            for k in &ext.keys {
                let mut args = vec![lua_quote(&inst.exe)];
                for a in &k.args {
                    args.push(lua_quote(a));
                }
                body.push_str(&format!(
                    "keys[#keys+1] = {{ key = {k}, mods = {m}, action = wezterm.action.SpawnCommandInNewTab {{ args = {{ {args} }} }} }}\n",
                    k = lua_quote(&k.key),
                    m = lua_quote(&k.mods),
                    args = args.join(", "),
                ));
            }
        }

        // Lua 片段：文件里 return 一个 function(config, wezterm)
        if inst.kind == "lua" {
            body.push_str(&format!(
                "do\n  local ok, fn = pcall(dofile, {p})\n  if ok and type(fn) == 'function' then apply[#apply+1] = fn end\nend\n",
                p = lua_quote(&inst.path)
            ));
        }
        body.push('\n');
    }

    body.push_str("return { keys = keys, apply = apply }\n");
    atomic_write(&ext_lua_path(), &body)?;
    Ok(ext_lua_path())
}

// ---------------- 安装 / 卸载 ----------------

fn install_binary(ext: &Extension, version: &str) -> Result<Installed, String> {
    let bin = ext
        .binary
        .as_ref()
        .ok_or_else(|| format!("{} 没有 binary 段，不能按二进制安装", ext.name))?;
    let asset = bin.asset.replace("{version}", version);
    let url = bin.url.replace("{version}", version).replace("{asset}", &asset);

    let pkg_dir = bin_dir().join(&ext.name);
    if pkg_dir.exists() {
        fs::remove_dir_all(&pkg_dir).map_err(|e| format!("清理旧版本失败: {e}"))?;
    }
    fs::create_dir_all(cache_dir()).map_err(|e| format!("创建缓存目录失败: {e}"))?;
    let tmp_zip = cache_dir().join(&asset);

    eprintln!("下载 {url}");
    curl_to(&url, &tmp_zip)?;

    if !bin.sha256.is_empty() {
        let got = sha256_file(&tmp_zip)?;
        if !got.eq_ignore_ascii_case(&bin.sha256) {
            return Err(format!(
                "sha256 不匹配！\n  期望 {}\n  实际 {got}",
                bin.sha256
            ));
        }
        eprintln!("sha256 校验通过");
    }

    eprintln!("解压到 {}", pkg_dir.display());
    extract_zip(&tmp_zip, &pkg_dir)?;
    let _ = fs::remove_file(&tmp_zip);

    // 找到 exe（可能在子目录里）
    let exe = find_file(&pkg_dir, &bin.exe)
        .ok_or_else(|| format!("压缩包里没找到 {}", bin.exe))?;
    Ok(Installed {
        version: version.to_string(),
        kind: "binary".into(),
        path: pkg_dir.to_string_lossy().to_string(),
        exe: exe.to_string_lossy().to_string(),
    })
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if p.is_file() && p.file_name().map(|n| n == name).unwrap_or(false) {
            return Some(p);
        }
        if p.is_dir() {
            if let Some(found) = find_file(&p, name) {
                return Some(found);
            }
        }
    }
    None
}

fn install_source(ext: &Extension, version: &str) -> Result<Installed, String> {
    let src = ext
        .source
        .as_ref()
        .ok_or_else(|| format!("{} 没有 source 段", ext.name))?;
    let work = src_dir().join(&ext.name);
    if work.exists() {
        fs::remove_dir_all(&work).map_err(|e| format!("清理旧源码失败: {e}"))?;
    }
    fs::create_dir_all(src_dir()).map_err(|e| format!("创建源码目录失败: {e}"))?;

    let tag = format!("v{version}");
    eprintln!("克隆 {} @ {tag}", src.repo);
    let mut clone = Command::new("git");
    clone.args([
        "clone",
        "--depth",
        "1",
        "--branch",
        &tag,
        &src.repo,
        &work.to_string_lossy(),
    ]);
    if run(&mut clone).is_err() {
        eprintln!("分支 {tag} 不可用，退回到默认分支");
        if work.exists() {
            let _ = fs::remove_dir_all(&work);
        }
        run(Command::new("git").args(["clone", "--depth", "1", &src.repo, &work.to_string_lossy()]))?;
    }

    eprintln!("cargo build --release");
    run(Command::new("cargo")
        .current_dir(&work)
        .args(["build", "--release"]))?;

    let crate_name = src.crate_.clone().unwrap_or_else(|| ext.name.clone());
    let built = work.join("target/release").join(format!("{crate_name}.exe"));
    if !built.exists() {
        return Err(format!("构建完成但没找到 {}", built.display()));
    }
    Ok(Installed {
        version: version.to_string(),
        kind: "source".into(),
        path: work.to_string_lossy().to_string(),
        exe: built.to_string_lossy().to_string(),
    })
}

fn install_lua(ext: &Extension, version: &str) -> Result<Installed, String> {
    let url = ext
        .lua
        .as_ref()
        .ok_or_else(|| format!("{} 没有 lua 段", ext.name))?;
    fs::create_dir_all(lua_dir()).map_err(|e| format!("创建 lua 目录失败: {e}"))?;
    let dest = lua_dir().join(format!("{}.lua", ext.name));
    eprintln!("下载 {url}");
    curl_to(url, &dest)?;
    Ok(Installed {
        version: version.to_string(),
        kind: "lua".into(),
        path: dest.to_string_lossy().to_string(),
        exe: String::new(),
    })
}

/// 没显式指定安装方式时：已装的按它原来的方式；没装的优先 binary，其次 lua
fn mode_for(ext: &Extension, installed: Option<&Installed>) -> InstallMode {
    if let Some(i) = installed {
        return match i.kind.as_str() {
            "lua" => InstallMode::Lua,
            "source" => InstallMode::Source,
            _ => InstallMode::Binary,
        };
    }
    if ext.binary.is_some() {
        InstallMode::Binary
    } else if ext.lua.is_some() {
        InstallMode::Lua
    } else {
        InstallMode::Source
    }
}

fn do_install(
    name: &str,
    mode: Option<InstallMode>,
    index: &Index,
    state: &mut State,
) -> Result<(), String> {
    let ext = index
        .extensions
        .get(name)
        .ok_or_else(|| format!("索引里没有扩展 {name}"))?
        .clone();
    let version = ext.version.clone();
    let mode = mode.unwrap_or_else(|| mode_for(&ext, state.extensions.get(name)));
    let installed = match mode {
        InstallMode::Binary => install_binary(&ext, &version)?,
        InstallMode::Source => install_source(&ext, &version)?,
        InstallMode::Lua => install_lua(&ext, &version)?,
    };
    println!(
        "已安装 {} {}（{}）-> {}",
        ext.name, version, installed.kind, installed.exe
    );
    state.extensions.insert(ext.name.clone(), installed);
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum InstallMode {
    Binary,
    Source,
    Lua,
}

// ---------------- 命令 ----------------

fn cmd_list(index: &Index, state: &State, source: &str) {
    println!("索引：{source}");
    println!(
        "{:<14} {:<9} {:<10} {}",
        "扩展", "版本", "状态", "说明"
    );
    for (name, ext) in &index.extensions {
        let (mark, ver) = match state.extensions.get(name) {
            Some(i) => (
                if i.version == ext.version {
                    "已安装"
                } else {
                    "可更新"
                },
                i.version.clone(),
            ),
            None => ("未安装", String::new()),
        };
        let mut flags = String::new();
        if ext.binary.is_some() {
            flags.push_str("bin ");
        }
        if ext.source.is_some() {
            flags.push_str("src ");
        }
        if ext.lua.is_some() {
            flags.push_str("lua ");
        }
        println!(
            "{:<14} {:<9} {:<10} {} [{}]",
            name,
            if ver.is_empty() { &ext.version } else { &ver },
            mark,
            ext.display,
            flags.trim()
        );
    }
}

fn cmd_install(
    names: &[String],
    mode: Option<InstallMode>,
    index: &Index,
    state: &mut State,
) -> Result<(), String> {
    if names.is_empty() {
        return Err("用法: wezext install <name>...".into());
    }
    for name in names {
        do_install(name, mode, index, state)?;
    }
    save_state(state)?;
    let path = regenerate_lua(state, index)?;
    println!("已更新 {}", path.display());
    Ok(())
}

fn cmd_remove(names: &[String], index: &Index, state: &mut State) -> Result<(), String> {
    for name in names {
        match state.extensions.remove(name) {
            Some(i) => {
                let p = PathBuf::from(&i.path);
                if i.kind == "lua" {
                    let _ = fs::remove_file(&p);
                } else if p.exists() {
                    let _ = fs::remove_dir_all(&p);
                }
                println!("已卸载 {name} {}", i.version);
            }
            None => println!("{name} 没装，跳过"),
        }
    }
    save_state(state)?;
    let path = regenerate_lua(state, index)?;
    println!("已更新 {}", path.display());
    Ok(())
}

fn cmd_doctor(index: &Index, state: &State) -> i32 {
    let mut problems = 0;
    println!("wezext 目录 : {}", root_dir().display());
    println!("索引来源    : {}", index_source());
    println!("扩展配置    : {}", ext_lua_path().display());
    println!();

    for (name, inst) in &state.extensions {
        if inst.kind == "lua" {
            let p = PathBuf::from(&inst.path);
            if p.is_file() {
                println!("✓ {name} {} (lua)", inst.version);
            } else {
                println!("✗ {name} lua 文件不见了: {}", inst.path);
                problems += 1;
            }
            continue;
        }
        let p = PathBuf::from(&inst.exe);
        if p.is_file() {
            println!("✓ {name} {} ({})", inst.version, inst.kind);
        } else {
            println!("✗ {name} 可执行文件不见了: {}", inst.exe);
            problems += 1;
        }
        match index.extensions.get(name) {
            Some(ext) if ext.version != inst.version => {
                println!("  ↑ 索引里是 {}，可以 wezext update {name}", ext.version);
            }
            Some(_) => {}
            None => {
                println!("  ? 索引里已经没有这个扩展了");
                problems += 1;
            }
        }
    }

    if state.extensions.is_empty() {
        println!("（还没装任何扩展）");
    }

    println!();
    // 检查主配置有没有接入
    match fs::read_to_string(main_config_path()) {
        Ok(cfg) => {
            if cfg.contains(".wezterm-ext.lua") {
                println!("✓ 主配置已接入 .wezterm-ext.lua");
            } else {
                println!("✗ 主配置里没找到 .wezterm-ext.lua 的 dofile，扩展不会生效");
                println!("  在 ~/.wezterm.lua 的 return config 之前加一行：");
                println!("    {INCLUDE_HINT}");
                problems += 1;
            }
        }
        Err(e) => {
            println!("✗ 读不到 {}: {e}", main_config_path().display());
            problems += 1;
        }
    }

    problems as i32
}

fn main() {
    cleanup_old_binaries();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("list");
    let rest: Vec<String> = args.iter().skip(1).cloned().collect();

    let build_mode = rest.iter().any(|a| a == "--build");
    let lua_mode = rest.iter().any(|a| a == "--lua");
    let names: Vec<String> = rest
        .iter()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .collect();

    let need_index = matches!(
        cmd,
        "list" | "install" | "update" | "build" | "remove" | "doctor"
    );
    let index = if need_index {
        match load_index(rest.iter().any(|a| a == "--refresh")) {
            Ok((i, _)) => Some(i),
            Err(e) => {
                eprintln!("加载索引失败: {e}");
                std::process::exit(1);
            }
        }
    } else {
        None
    };
    let mut state = load_state();

    let code = match cmd {
        "list" | "ls" => {
            let (index, src) = load_index(rest.iter().any(|a| a == "--refresh"))
                .unwrap_or_else(|e| {
                    eprintln!("加载索引失败: {e}");
                    std::process::exit(1);
                });
            cmd_list(&index, &state, &src);
            0
        }
        "install" | "i" => {
            let mode = if build_mode {
                Some(InstallMode::Source)
            } else if lua_mode {
                Some(InstallMode::Lua)
            } else {
                None
            };
            match cmd_install(&names, mode, index.as_ref().unwrap(), &mut state) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("安装失败: {e}");
                    1
                }
            }
        }
        "build" => match cmd_install(&names, Some(InstallMode::Source), index.as_ref().unwrap(), &mut state) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("构建失败: {e}");
                1
            }
        },
        "update" | "up" => {
            let targets: Vec<String> = if names.is_empty() {
                state
                    .extensions
                    .keys()
                    .filter(|n| {
                        index
                            .as_ref()
                            .map(|i| i.extensions.contains_key(*n))
                            .unwrap_or(false)
                    })
                    .cloned()
                    .collect()
            } else {
                names.clone()
            };
            let mode = if build_mode {
                Some(InstallMode::Source)
            } else {
                None
            };
            if targets.is_empty() {
                println!("没有需要更新的扩展");
                0
            } else {
                match cmd_install(&targets, mode, index.as_ref().unwrap(), &mut state) {
                    Ok(()) => 0,
                    Err(e) => {
                        eprintln!("更新失败: {e}");
                        1
                    }
                }
            }
        }
        "remove" | "rm" | "uninstall" => {
            let index = index.as_ref().unwrap();
            match cmd_remove(&names, index, &mut state) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("卸载失败: {e}");
                    1
                }
            }
        }
        "doctor" | "dr" => cmd_doctor(index.as_ref().unwrap(), &state),
        "self-update" | "upgrade" => cmd_self_update(rest.iter().any(|a| a == "--force")),
        "index" => match load_index(true) {
            Ok((i, src)) => {
                println!("索引来源: {src}");
                println!("扩展数量: {}", i.extensions.len());
                0
            }
            Err(e) => {
                eprintln!("刷新失败: {e}");
                1
            }
        },
        "-h" | "--help" | "help" => {
            println!("{}", HELP);
            0
        }
        other => {
            eprintln!("未知命令: {other}\n");
            eprintln!("{HELP}");
            2
        }
    };
    std::process::exit(code);
}

const HELP: &str = "\
wezext —— wezterm 扩展管理器

用法: wezext <命令> [参数]

命令:
  list                   列出索引里的扩展和安装状态（--refresh 强制刷新）
  install <name>...      安装扩展（默认下载二进制）
                          --build  从源码构建（需要 git + cargo）
                          --lua    只安装 Lua 片段
  update [name]...       更新（不写名字就更新全部已装的）
  build <name>...        从源码构建并安装
  remove <name>...       卸载
  doctor                 自检：文件、版本、主配置是否已接入
  self-update [--force]  把 wezext 自己更新到最新 release
  index                  刷新索引缓存

环境变量:
  WEZEXT_INDEX   覆盖索引来源：本地目录 / 本地文件 / URL
";

// ---------------- self-update ----------------

const SELF_REPO: &str = "wezext-community/wezext";
const SELF_VERSION: &str = env!("CARGO_PKG_VERSION");

fn parse_version(v: &str) -> Vec<u32> {
    v.trim_start_matches('v')
        .split('.')
        .filter_map(|p| p.parse::<u32>().ok())
        .collect()
}

/// 启动时清理上次升级留下的 .old（当时它被占用，删不掉）
fn cleanup_old_binaries() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let old = exe.with_extension("exe.old");
    let _ = fs::remove_file(old);
    let old2 = PathBuf::from(format!("{}.old", exe.display()));
    let _ = fs::remove_file(old2);
}

fn cmd_self_update(force: bool) -> i32 {
    println!("当前版本: {SELF_VERSION}");

    let api = format!("https://api.github.com/repos/{SELF_REPO}/releases/latest");
    let tmp = cache_dir().join("self-release.json");
    if let Err(e) = fs::create_dir_all(cache_dir()) {
        eprintln!("创建缓存目录失败: {e}");
        return 1;
    }

    let mut ok = false;
    for attempt in 1..=4 {
        match curl_to(&api, &tmp) {
            Ok(()) => {
                ok = true;
                break;
            }
            Err(e) => eprintln!("  第 {attempt} 次获取 release 信息失败: {e}"),
        }
        std::thread::sleep(std::time::Duration::from_secs(3));
    }
    if !ok {
        eprintln!("获取最新版本失败（网络？）");
        return 1;
    }

    let text = match fs::read_to_string(&tmp) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("读取 release 信息失败: {e}");
            return 1;
        }
    };
    let json: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("解析 release 信息失败: {e}");
            return 1;
        }
    };
    let tag = json["tag_name"].as_str().unwrap_or_default().to_string();
    let latest = tag.trim_start_matches('v').to_string();
    println!("最新版本: {latest}（{tag}）");

    if !force && parse_version(&latest) <= parse_version(SELF_VERSION) {
        println!("已是最新，无需更新（想强制重装加 --force）");
        return 0;
    }

    // 找匹配的资产
    let wanted = format!("wezext-v{latest}-x86_64-pc-windows-msvc.zip");
    let mut url = None;
    if let Some(assets) = json["assets"].as_array() {
        for a in assets {
            if a["name"].as_str() == Some(wanted.as_str()) {
                url = a["browser_download_url"].as_str().map(|s| s.to_string());
                break;
            }
        }
    }
    let Some(url) = url else {
        eprintln!("这个 release 里没有 {wanted}，跳过");
        return 1;
    };

    let zip = cache_dir().join(&wanted);
    let unpack = cache_dir().join("self-new");
    let _ = fs::remove_dir_all(&unpack);
    if let Err(e) = curl_to(&url, &zip) {
        eprintln!("下载失败: {e}");
        return 1;
    }
    if let Err(e) = extract_zip(&zip, &unpack) {
        eprintln!("解压失败: {e}");
        return 1;
    }
    let Some(new_exe) = find_file(&unpack, "wezext.exe") else {
        eprintln!("压缩包里没有 wezext.exe");
        return 1;
    };

    let Ok(current) = std::env::current_exe() else {
        eprintln!("拿不到当前 exe 路径");
        return 1;
    };
    let backup = PathBuf::from(format!("{}.old", current.display()));

    // 正在运行的 exe 不能覆盖，但可以改名
    if let Err(e) = fs::rename(&current, &backup) {
        eprintln!("无法重命名当前 exe（{e}），是不是装在只读目录？");
        return 1;
    }
    if let Err(e) = fs::copy(&new_exe, &current) {
        eprintln!("复制新版本失败（{e}），正在回滚");
        let _ = fs::rename(&backup, &current);
        return 1;
    }
    let _ = fs::remove_file(&zip);
    let _ = fs::remove_dir_all(&unpack);

    println!(
        "已更新到 {latest}。重新运行 wezext 生效（旧版本留在 {}，下次启动自动清理）",
        backup.display()
    );
    0
}
