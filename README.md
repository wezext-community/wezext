# wezext

wezterm 扩展管理器：从一个索引仓库安装 / 更新 / 卸载扩展（TUI 小工具、Lua 片段）。

```
wezext list                 # 看索引里有什么、装了没
wezext install wezfont      # 下载二进制安装
wezext install --build wezfont   # 从源码构建安装（需要 git + cargo）
wezext update               # 更新全部已装扩展
wezext remove wezfont
wezext doctor               # 自检：文件、版本、配置是否接入
```

## 安装目录

```
%LOCALAPPDATA%\wezext\
  state.json          已安装的扩展与版本
  bin\<name>\         二进制扩展
  lua\<name>.lua      Lua 片段扩展
  src\<name>\         源码构建的扩展
```

扩展的键位不会写进你的主配置，而是生成 `~/.wezterm-ext.lua`，
由 `~/.wezterm.lua` 末尾这样一行引入：

```lua
pcall(dofile, wezterm.home_dir .. '/.wezterm-ext.lua')
```

`wezext doctor` 会检查这行在不在。

## 索引

默认拉 `https://raw.githubusercontent.com/wezext-community/wezext-index/main/index.json`。
可以用 `WEZEXT_INDEX` 覆盖成本地目录 / 文件 / URL，方便调试与离线。

## 依赖

只用 Windows 自带的 `curl.exe`（下载）和 `tar.exe`（解压 zip）。
源码构建模式下额外需要 `git` 和 `cargo`。
