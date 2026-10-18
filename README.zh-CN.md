# wezext

[English](README.md) | **中文**

[WezTerm](https://wezterm.org) 的扩展管理器：从一个集中索引安装、更新、卸载小工具和
Lua 片段 —— 不用手改主配置。

```
wezext list                      # 索引里有什么 / 装了没
wezext install wezfont          # 下载预编译二进制并安装
wezext install --build wezfont  # 改为从源码构建（需要 git + cargo）
wezext install --lua some-ext   # 只装 Lua 片段
wezext update                    # 更新全部已安装的扩展
wezext remove wezfont
wezext doctor                    # 自检：文件、版本、主配置是否接入
```

## 工作方式

扩展永远不碰你的主配置。`wezext` 只写一个生成文件 `~/.wezterm-ext.lua`，内容是：

* 调用已安装可执行文件的键位绑定，以及
* 需要应用的 Lua 扩展函数列表。

`~/.wezterm.lua` 末尾只要一行：

```lua
pcall(dofile, wezterm.home_dir .. '/.wezterm-ext.lua')
```

`wezext doctor` 会告诉你这行在不在。

## 安装目录

```
%LOCALAPPDATA%\wezext\
  state.json          已安装的扩展与版本
  bin\<name>\         二进制扩展
  lua\<name>.lua      Lua 片段扩展
  src\<name>\         源码构建的扩展
```

不会往 `PATH` 里塞任何东西；键位绑定直接用绝对路径指向安装好的二进制。

## 索引

默认从
`https://raw.githubusercontent.com/wezext-community/wezext-index/main/index.json`
拉取并本地缓存。可以用 `WEZEXT_INDEX` 覆盖为本地目录 / 文件 / URL —— 方便调试和离线。

## 环境要求

Windows 10/11。下载用系统自带 `curl.exe`，解压用系统自带 `tar.exe`，无额外依赖。
源码构建模式额外需要 `git` 和 `cargo`。

## 构建

```bash
cargo build --release
# -> target/release/wezext.exe
```

## 相关仓库

| 仓库 | 用途 |
| --- | --- |
| [wezext-index](https://github.com/wezext-community/wezext-index) | 索引本体（每个扩展一个 JSON） |
| [wezfont](https://github.com/wezext-community/wezfont) | 字体选择 TUI |
| [wezlauncher](https://github.com/wezext-community/wezlauncher) | 启动器 TUI |
