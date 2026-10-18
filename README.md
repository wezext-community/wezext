# wezext

**English** | [中文](README.zh-CN.md)

An extension manager for [WezTerm](https://wezterm.org): install, update and remove
small tools and Lua snippets from a central index — without hand-editing your config.

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
