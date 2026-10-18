# wezext

**English** | [中文](README.zh-CN.md)

An extension manager for [WezTerm](https://wezterm.org): install, update and remove
small tools and Lua snippets from a central index — without hand-editing your config.

## Install

```powershell
powershell -c "irm https://raw.githubusercontent.com/wezext-community/wezext/main/install.ps1 | iex"
```

The script resolves the latest release, downloads the zip, verifies its sha256,
unpacks it to `%LOCALAPPDATA%\wezextin` and adds that directory to your user `PATH`.
Knobs (env vars, since `iex` can't take arguments): `WEZEXT_VERSION`, `WEZEXT_DIR`,
`WEZEXT_NO_PATH=1`.

### Two install modes

| Mode | How | Needs |
| --- | --- | --- |
| Binary (default) | download the release zip, verify sha256, unzip | nothing |
| From source | `git clone` + `cargo build --release`, then install the built exe | `git` + `cargo` |

```powershell
# from source (env var, because `iex` can't take arguments)
$env:WEZEXT_FROM_SOURCE=1; irm https://raw.githubusercontent.com/wezext-community/wezext/main/install.ps1 | iex

# or, running the file directly
pwsh -File install.ps1 -Source
```

The script prints plain ASCII status lines only — it does not depend on any third-party
project, font or prompt framework.

Or grab `wezext-*-x86_64-pc-windows-msvc.zip` from
[Releases](https://github.com/wezext-community/wezext/releases) and unzip it anywhere.

```
wezext list                      # what's in the index, and what's installed
wezext install wezfont          # download the prebuilt binary and install it
wezext install --build wezfont  # build from source instead (needs git + cargo)
wezext install --lua some-ext   # Lua-only extension
wezext update                    # update everything installed
wezext remove wezfont
wezext doctor                    # sanity check: files, versions, config wiring
```

## How it works

Extensions never touch your main config. `wezext` writes a single generated file,
`~/.wezterm-ext.lua`, containing:

* key bindings that launch installed executables, and
* a list of Lua extension functions to apply.

Your `~/.wezterm.lua` only needs one line near the end:

```lua
pcall(dofile, wezterm.home_dir .. '/.wezterm-ext.lua')
```

`wezext doctor` tells you if that line is missing.

## Install layout

```
%LOCALAPPDATA%\wezext\
  state.json          installed extensions + versions
  bin\<name>\         binary extensions
  lua\<name>.lua      Lua-snippet extensions
  src\<name>\         source-built extensions
```

Nothing is added to `PATH`; key bindings reference the installed binaries by
absolute path.

## Index

By default the index is fetched from
`https://raw.githubusercontent.com/wezext-community/wezext-index/main/index.json`
and cached locally. Override it with `WEZEXT_INDEX` (a directory, a file, or a URL)
— handy for development and offline use.

## Requirements

Windows 10/11. Downloads use the built-in `curl.exe`, extraction uses the built-in
`tar.exe`; no extra dependencies. Source builds additionally need `git` and `cargo`.

## Building

```bash
cargo build --release
# -> target/release/wezext.exe
```

## Related repositories

| Repository | Purpose |
| --- | --- |
| [wezext-index](https://github.com/wezext-community/wezext-index) | the index itself (one JSON per extension) |
| [wezfont](https://github.com/wezext-community/wezfont) | font picker TUI |
| [wezlauncher](https://github.com/wezext-community/wezlauncher) | launcher TUI |
