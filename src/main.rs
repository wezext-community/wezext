// ============================================================
//  wezext - extension manager for WezTerm
//
//  Index: the wezext-index repo (index.json). Override it with WEZEXT_INDEX
//         (a local directory, a file, or a URL).
//  Layout: %LOCALAPPDATA%\wezext\{bin,lua,src}\<name>\
//  Config: generates ~/.wezterm-ext.lua, which ~/.wezterm.lua dofiles at the end
//          (wezext never edits your main config).
//
//  Usage: wezext <command>
//    (no args) / tui      open the interactive TUI
//    list                 list indexed extensions and install state
//    install <name>...    install (binary by default; --build source; --lua snippet)
//    remove <name>...     uninstall
//    update [name]...     update everything installed (or the given names)
//    build <name>...      build from source (needs a Rust toolchain)
//    doctor               self-check: files, versions, config wiring
//    self-update          update wezext itself
//    index                show the index source and refresh the cache
// ============================================================
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod tui;

// ---------------- data types ----------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct Index {
    #[serde(default)]
    schema: u32,
    #[serde(default)]
    extensions: BTreeMap<String, Extension>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct Extension {
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
    /// release asset name template, supports {version}
    asset: String,
    /// executable name inside the archive
    exe: String,
    /// download URL template, supports {version} / {asset}
    url: String,
    /// optional sha256 (lowercase hex)
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
pub(crate) struct State {
    #[serde(default)]
    extensions: BTreeMap<String, Installed>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct Installed {
    version: String,
    /// "binary" | "source" | "lua"
    kind: String,
    /// install directory (for lua extensions: the file path)
    path: String,
    #[serde(default)]
    exe: String,
}

// ---------------- paths ----------------

fn home() -> PathBuf {
    PathBuf::from(std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into()))
}

fn local_appdata() -> PathBuf {
    std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home().join("AppData/Local"))
}

pub(crate) fn root_dir() -> PathBuf {
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
pub(crate) fn ext_lua_path() -> PathBuf {
    home().join(".wezterm-ext.lua")
}
pub(crate) fn main_config_path() -> PathBuf {
    home().join(".wezterm.lua")
}

pub(crate) const INCLUDE_HINT: &str = "pcall(dofile, wezterm.home_dir .. '/.wezterm-ext.lua')";

// ---------------- index ----------------

pub(crate) fn index_source() -> String {
    std::env::var("WEZEXT_INDEX").unwrap_or_else(|_| {
        "https://raw.githubusercontent.com/wezext-community/wezext-index/main/index.json".to_string()
    })
}

pub(crate) fn load_index(refresh: bool) -> Result<(Index, String), String> {
    let src = index_source();

    // local directory
    let as_path = PathBuf::from(&src);
    if as_path.is_dir() {
        let file = as_path.join("index.json");
        let text = fs::read_to_string(&file)
            .map_err(|e| format!("failed to read {}: {e}", file.display()))?;
        let index: Index =
            serde_json::from_str(&text).map_err(|e| format!("failed to parse {}: {e}", file.display()))?;
        return Ok((index, src));
    }
    // local file
    if as_path.is_file() {
        let text = fs::read_to_string(&as_path)
            .map_err(|e| format!("failed to read {}: {e}", as_path.display()))?;
        let index: Index = serde_json::from_str(&text)
            .map_err(|e| format!("failed to parse {}: {e}", as_path.display()))?;
        return Ok((index, src));
    }

    // remote: cache it
    let cache = cache_dir().join("index.json");
    if !refresh && cache.is_file() {
        if let Ok(text) = fs::read_to_string(&cache) {
            if let Ok(index) = serde_json::from_str::<Index>(&text) {
                return Ok((index, format!("{src} (cached)")));
            }
        }
    }
    fs::create_dir_all(cache_dir()).map_err(|e| format!("failed to create the cache dir: {e}"))?;
    curl_to(&src, &cache)?;
    let text = fs::read_to_string(&cache).map_err(|e| format!("failed to read the cache: {e}"))?;
    let index: Index = serde_json::from_str(&text).map_err(|e| format!("failed to parse the index: {e}"))?;
    Ok((index, src))
}

// ---------------- external tools ----------------

fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd.output().map_err(|e| format!("failed to spawn a process: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "command failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// download with the bundled curl.exe
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
    .map_err(|e| format!("failed to download {url}: {e}"))
}

/// extract the zip with the bundled tar.exe
fn extract_zip(zip: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| format!("failed to create {}: {e}", dest.display()))?;

    // Prefer bsdtar; some tools produce zips it cannot read (e.g. Compress-Archive),
    // in that case fall back to PowerShell Expand-Archive.
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
    .map_err(|e| format!("extract failed (both tar and Expand-Archive failed): {e}"))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let data = fs::read(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>())
}

// ---------------- state ----------------

pub(crate) fn load_state() -> State {
    fs::read_to_string(state_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub(crate) fn save_state(state: &State) -> Result<(), String> {
    fs::create_dir_all(root_dir()).map_err(|e| format!("failed to create {}: {e}", root_dir().display()))?;
    let text = serde_json::to_string_pretty(state).map_err(|e| format!("failed to serialize state: {e}"))?;
    atomic_write(&state_path(), &text)
}

/// write to a temp file then rename, so nobody reads a half-written file
fn atomic_write(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text).map_err(|e| format!("failed to write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("failed to replace {}: {e}", path.display()))
}

// ---------------- generate ~/.wezterm-ext.lua ----------------

fn lua_quote(s: &str) -> String {
    // Lua long strings avoid all escaping pitfalls
    format!("[[{}]]", s)
}

pub(crate) fn regenerate_lua(state: &State, index: &Index) -> Result<PathBuf, String> {
    let mut body = String::new();
    body.push_str("-- generated by wezext - do not edit, it gets overwritten\n");
    body.push_str("-- add this at the end of your main config: ");
    body.push_str(INCLUDE_HINT);
    body.push_str("\n\nlocal wezterm = require 'wezterm'\n\nlocal keys = {}\nlocal apply = {}\n\n");

    for (name, inst) in &state.extensions {
        let Some(ext) = index.extensions.get(name) else {
            continue;
        };
        body.push_str(&format!("-- {} {}\n", name, inst.version));

        // keys: launch the installed executable
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

        // lua snippet: the file returns function(config, wezterm)
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

// ---------------- install / remove ----------------

fn install_binary(ext: &Extension, version: &str) -> Result<Installed, String> {
    let bin = ext
        .binary
        .as_ref()
        .ok_or_else(|| format!("{} has no binary section", ext.name))?;
    let asset = bin.asset.replace("{version}", version);
    let url = bin.url.replace("{version}", version).replace("{asset}", &asset);

    let pkg_dir = bin_dir().join(&ext.name);
    if pkg_dir.exists() {
        fs::remove_dir_all(&pkg_dir).map_err(|e| format!("failed to clean the old version: {e}"))?;
    }
    fs::create_dir_all(cache_dir()).map_err(|e| format!("failed to create the cache dir: {e}"))?;
    let tmp_zip = cache_dir().join(&asset);

    eprintln!("downloading {url}");
    curl_to(&url, &tmp_zip)?;

    if !bin.sha256.is_empty() {
        let got = sha256_file(&tmp_zip)?;
        if !got.eq_ignore_ascii_case(&bin.sha256) {
            return Err(format!(
                "sha256 mismatch!\n  expected {}\n  actual   {got}",
                bin.sha256
            ));
        }
        eprintln!("sha256 verified");
    }

    eprintln!("extracting to {}", pkg_dir.display());
    extract_zip(&tmp_zip, &pkg_dir)?;
    let _ = fs::remove_file(&tmp_zip);

    // locate the exe (it may be in a subdirectory)
    let exe = find_file(&pkg_dir, &bin.exe)
        .ok_or_else(|| format!("{} not found in the archive", bin.exe))?;
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
        .ok_or_else(|| format!("{} has no source section", ext.name))?;
    let work = src_dir().join(&ext.name);
    if work.exists() {
        fs::remove_dir_all(&work).map_err(|e| format!("failed to clean the old source tree: {e}"))?;
    }
    fs::create_dir_all(src_dir()).map_err(|e| format!("failed to create the source dir: {e}"))?;

    let tag = format!("v{version}");
    eprintln!("cloning {} @ {tag}", src.repo);
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
        eprintln!("tag {tag} not available, falling back to the default branch");
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
        return Err(format!("build finished but {} is missing", built.display()));
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
        .ok_or_else(|| format!("{} has no lua section", ext.name))?;
    fs::create_dir_all(lua_dir()).map_err(|e| format!("failed to create the lua dir: {e}"))?;
    let dest = lua_dir().join(format!("{}.lua", ext.name));
    eprintln!("downloading {url}");
    curl_to(url, &dest)?;
    Ok(Installed {
        version: version.to_string(),
        kind: "lua".into(),
        path: dest.to_string_lossy().to_string(),
        exe: String::new(),
    })
}

/// Pick an install mode: reuse the installed kind, else prefer binary, then lua
pub(crate) fn mode_for(ext: &Extension, installed: Option<&Installed>) -> InstallMode {
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

pub(crate) fn do_install(
    name: &str,
    mode: Option<InstallMode>,
    index: &Index,
    state: &mut State,
) -> Result<(), String> {
    let ext = index
        .extensions
        .get(name)
        .ok_or_else(|| format!("extension {name} is not in the index"))?
        .clone();
    let version = ext.version.clone();
    let mode = mode.unwrap_or_else(|| mode_for(&ext, state.extensions.get(name)));
    let installed = match mode {
        InstallMode::Binary => install_binary(&ext, &version)?,
        InstallMode::Source => install_source(&ext, &version)?,
        InstallMode::Lua => install_lua(&ext, &version)?,
    };
    println!(
        "installed {} {} ({}) -> {}",
        ext.name, version, installed.kind, installed.exe
    );
    state.extensions.insert(ext.name.clone(), installed);
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum InstallMode {
    Binary,
    Source,
    Lua,
}

// ---------------- commands ----------------

fn cmd_list(index: &Index, state: &State, source: &str) {
    println!("index: {source}");
    println!(
        "{:<14} {:<9} {:<10} {}",
        "extension", "version", "state", "description"
    );
    for (name, ext) in &index.extensions {
        let (mark, ver) = match state.extensions.get(name) {
            Some(i) => (
                if i.version == ext.version {
                    "installed"
                } else {
                    "update available"
                },
                i.version.clone(),
            ),
            None => ("not installed", String::new()),
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
        return Err("usage: wezext install <name>...".into());
    }
    for name in names {
        do_install(name, mode, index, state)?;
    }
    save_state(state)?;
    let path = regenerate_lua(state, index)?;
    println!("updated {}", path.display());
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
                println!("removed {name} {}", i.version);
            }
            None => println!("{name} is not installed, skipping"),
        }
    }
    save_state(state)?;
    let path = regenerate_lua(state, index)?;
    println!("updated {}", path.display());
    Ok(())
}

/// Build the doctor report (printed by the CLI, rendered by the TUI)
pub(crate) fn doctor_lines(index: &Index, state: &State) -> (Vec<String>, usize) {
    let mut lines: Vec<String> = Vec::new();
    let mut problems = 0;

    lines.push(format!("wezext dir  : {}", root_dir().display()));
    lines.push(format!("index source: {}", index_source()));
    lines.push(format!("ext config  : {}", ext_lua_path().display()));
    lines.push(String::new());

    for (name, inst) in &state.extensions {
        if inst.kind == "lua" {
            let p = PathBuf::from(&inst.path);
            if p.is_file() {
                lines.push(format!("[ok] {name} {} (lua)", inst.version));
            } else {
                lines.push(format!("[!!] {name} lua file is missing: {}", inst.path));
                problems += 1;
            }
            continue;
        }
        let p = PathBuf::from(&inst.exe);
        if p.is_file() {
            lines.push(format!("[ok] {name} {} ({})", inst.version, inst.kind));
        } else {
            lines.push(format!("[!!] {name} executable is missing: {}", inst.exe));
            problems += 1;
        }
        match index.extensions.get(name) {
            Some(ext) if ext.version != inst.version => {
                lines.push(format!("     ^ index has {}, update available", ext.version));
            }
            Some(_) => {}
            None => {
                lines.push("     ? this extension is no longer in the index".to_string());
                problems += 1;
            }
        }
    }

    if state.extensions.is_empty() {
        lines.push("(no extensions installed yet)".to_string());
    }

    lines.push(String::new());
    match fs::read_to_string(main_config_path()) {
        Ok(cfg) => {
            if cfg.contains(".wezterm-ext.lua") {
                lines.push("[ok] main config dofiles .wezterm-ext.lua".to_string());
            } else {
                lines.push("[!!] main config does not dofile .wezterm-ext.lua, extensions will not load".to_string());
                lines.push(format!("     add this before `return config` in ~/.wezterm.lua: {INCLUDE_HINT}"));
                problems += 1;
            }
        }
        Err(e) => {
            lines.push(format!("[!!] cannot read {}: {e}", main_config_path().display()));
            problems += 1;
        }
    }

    (lines, problems)
}

fn cmd_doctor(index: &Index, state: &State) -> i32 {
    let (lines, problems) = doctor_lines(index, state);
    for line in lines {
        println!("{line}");
    }
    problems as i32
}

fn main() {
    cleanup_old_binaries();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("list");

    // no arguments / explicit `tui`: open the interactive UI
    if args.is_empty() || cmd == "tui" {
        std::process::exit(tui::run());
    }
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
                eprintln!("failed to load index: {e}");
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
                    eprintln!("failed to load index: {e}");
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
                    eprintln!("install failed: {e}");
                    1
                }
            }
        }
        "build" => match cmd_install(&names, Some(InstallMode::Source), index.as_ref().unwrap(), &mut state) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("build failed: {e}");
                1
            }
        },
        "update" | "up" => {
            let idx = index.as_ref().unwrap();
            let force = rest.iter().any(|a| a == "--force");

            // 候选：指定了名字就用名字，否则是所有已装的
            let candidates: Vec<String> = if names.is_empty() {
                state.extensions.keys().cloned().collect()
            } else {
                names.clone()
            };

            // 只保留"索引里有"且（版本不同 或 --force）的
            let mut targets: Vec<String> = Vec::new();
            for name in &candidates {
                let Some(ext) = idx.extensions.get(name) else {
                    println!("{name}: no longer in the index, skipping");
                    continue;
                };
                match state.extensions.get(name) {
                    Some(inst) if !force && inst.version == ext.version => {
                        println!("{name}: already up to date ({})", inst.version);
                    }
                    Some(inst) => {
                        println!("{name}: {} -> {}", inst.version, ext.version);
                        targets.push(name.clone());
                    }
                    None => {
                        println!("{name}: not installed, skipping (use `wezext install {name}`)");
                    }
                }
            }

            let mode = if build_mode {
                Some(InstallMode::Source)
            } else {
                None
            };
            if targets.is_empty() {
                println!("nothing to update");
                0
            } else {
                match cmd_install(&targets, mode, idx, &mut state) {
                    Ok(()) => 0,
                    Err(e) => {
                        eprintln!("update failed: {e}");
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
                    eprintln!("remove failed: {e}");
                    1
                }
            }
        }
        "doctor" | "dr" => cmd_doctor(index.as_ref().unwrap(), &state),
        "self-update" | "upgrade" => cmd_self_update(rest.iter().any(|a| a == "--force")),
        "index" => match load_index(true) {
            Ok((i, src)) => {
                println!("index source: {src}");
                println!("extensions: {}", i.extensions.len());
                0
            }
            Err(e) => {
                eprintln!("refresh failed: {e}");
                1
            }
        },
        "-V" | "--version" | "version" => {
            println!("wezext {SELF_VERSION}");
            0
        }
        "-h" | "--help" | "help" => {
            println!("{}", HELP);
            0
        }
        other => {
            eprintln!("unknown command: {other}\n");
            eprintln!("{HELP}");
            2
        }
    };
    std::process::exit(code);
}

const HELP: &str = "wezext - extension manager for WezTerm

usage: wezext [command] [args]

commands:
  (no args) / tui         open the interactive TUI
  list                    list indexed extensions and install state (--refresh)
  install <name>...       install extensions (binary by default)
                            --build  build from source (needs git + cargo)
                            --lua    install a Lua-snippet extension
  update [name]...        update installed extensions (skips ones already
                          at the index version; add --force to reinstall)
  build <name>...         build from source and install
  remove <name>...        uninstall
  doctor                  self-check: files, versions, config wiring
  self-update [--force]   update wezext itself
  index                   refresh the cached index

environment:
  WEZEXT_INDEX   override the index source: a directory, a file, or a URL
";

// ---------------- self-update ----------------

const SELF_REPO: &str = "wezext-community/wezext";
pub(crate) const SELF_VERSION: &str = env!("CARGO_PKG_VERSION");

fn parse_version(v: &str) -> Vec<u32> {
    v.trim_start_matches('v')
        .split('.')
        .filter_map(|p| p.parse::<u32>().ok())
        .collect()
}

/// clean up the .old file left by the previous self-update
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
    println!("current: {SELF_VERSION}");

    let api = format!("https://api.github.com/repos/{SELF_REPO}/releases/latest");
    let tmp = cache_dir().join("self-release.json");
    if let Err(e) = fs::create_dir_all(cache_dir()) {
        eprintln!("failed to create cache dir: {e}");
        return 1;
    }

    let mut ok = false;
    for attempt in 1..=4 {
        match curl_to(&api, &tmp) {
            Ok(()) => {
                ok = true;
                break;
            }
            Err(e) => eprintln!("  attempt {attempt}: failed to fetch release info: {e}"),
        }
        std::thread::sleep(std::time::Duration::from_secs(3));
    }
    if !ok {
        eprintln!("failed to fetch the latest version (network?)");
        return 1;
    }

    let text = match fs::read_to_string(&tmp) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("failed to read release info: {e}");
            return 1;
        }
    };
    let json: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("failed to parse release info: {e}");
            return 1;
        }
    };
    let tag = json["tag_name"].as_str().unwrap_or_default().to_string();
    let latest = tag.trim_start_matches('v').to_string();
    println!("latest: {latest} ({tag})");

    if !force && parse_version(&latest) <= parse_version(SELF_VERSION) {
        println!("already up to date (use --force to reinstall)");
        return 0;
    }

    // find the matching asset
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
        eprintln!("release has no {wanted}, skipping");
        return 1;
    };

    let zip = cache_dir().join(&wanted);
    let unpack = cache_dir().join("self-new");
    let _ = fs::remove_dir_all(&unpack);
    if let Err(e) = curl_to(&url, &zip) {
        eprintln!("download failed: {e}");
        return 1;
    }
    if let Err(e) = extract_zip(&zip, &unpack) {
        eprintln!("extract failed: {e}");
        return 1;
    }
    let Some(new_exe) = find_file(&unpack, "wezext.exe") else {
        eprintln!("the archive has no wezext.exe");
        return 1;
    };

    let Ok(current) = std::env::current_exe() else {
        eprintln!("cannot resolve the current exe path");
        return 1;
    };
    let backup = PathBuf::from(format!("{}.old", current.display()));

    // a running exe cannot be overwritten, but it can be renamed
    if let Err(e) = fs::rename(&current, &backup) {
        eprintln!("cannot rename the running exe ({e}) - is it in a read-only directory?");
        return 1;
    }
    if let Err(e) = fs::copy(&new_exe, &current) {
        eprintln!("failed to copy the new version ({e}), rolling back");
        let _ = fs::rename(&backup, &current);
        return 1;
    }
    let _ = fs::remove_file(&zip);
    let _ = fs::remove_dir_all(&unpack);

    println!(
        "updated to {latest}. Restart wezext to use it (old binary kept at {}, removed on next start)",
        backup.display()
    );
    0
}
