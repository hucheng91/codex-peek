# codex-peek

**看清你粘贴进 [OpenAI Codex CLI](https://github.com/openai/codex) 的图片。**

Codex 的终端界面里，粘贴的图片只显示成 `[Image #1]`、`[Image #2]` 这样的占位符。一次贴 3 到 4 张截图时，没法确认哪张是哪张。`codex-peek` 是一个很小的置顶悬浮窗，按 Codex 的编号顺序显示每张粘贴图片的缩略图。

[English](README.md)

<p align="center">
  <a href="docs/demo-real-codex.mp4"><img src="docs/demo-real-codex.gif" width="900" alt="codex-peek 与真实 Codex CLI"></a>
  <br><sub>真实 Codex CLI 0.162.1：会话开始自动打开窗口，3 次 Ctrl+V，放大，清空，/quit 后自动关闭 · <a href="docs/demo-real-codex.mp4">MP4（28 秒）</a></sub>
</p>

<p align="center">
  <img src="docs/screenshot.png" width="450" alt="codex-peek 截图">
  &nbsp;
  <img src="docs/demo.gif" width="450" alt="codex-peek 演示">
</p>

- 使用系统自带的 WebView（macOS 用 WKWebView，Windows 用 WebView2，Linux 用 WebKitGTK），基于 wry + tao。不用 Electron，Linux x64 实测二进制约 **0.8 MB**。
- 缩略图按出现顺序编号 `Image #1..N`，点击放大，再点一次返回。
- **Clear** 按钮清空列表，编号从 1 重新开始。也可以用 `--idle` 在一段时间没有新图后自动清空。
- 无边框小窗口，拖动标题栏移动，会记住上次的位置。
- **Codex 插件**：Codex 会话开始时自动打开窗口，会话结束时自动关闭。

## 原理

Codex 粘贴剪贴板图片时，会先把图片写入临时文件（见 `codex-rs/tui/src/clipboard_paste.rs` 的 `paste_image_to_temp_png`）。文件用 `tempfile` 创建，前缀 `codex-clipboard-`，后缀 `.png`，放在 `std::env::temp_dir()`：

- macOS：`$TMPDIR`（如 `/var/folders/…/T/`）
- Windows：`%TEMP%`
- Linux：`$TMPDIR` 或 `/tmp`
- WSL：Codex 改用 PowerShell 读剪贴板，文件写到 **Windows** 临时目录，文件名是 `tmpXXXX.png`。

codex-peek 每 400 ms 检查一次这个目录，显示启动之后新出现的匹配文件。

## 安装

### 方式一：Codex 插件（推荐，自动启动和关闭）

```sh
codex plugin marketplace add hucheng91/codex-peek
codex plugin add codex-peek@codex-peek
```

插件通过两种方式让窗口跟随 Codex 会话：

1. **MCP 服务器（主要方式）**：Codex 在会话启动时就会启动插件的 MCP 服务器，退出时关闭它。插件运行 `node bin/codex-peek-mcp.js`，它会找到或下载二进制，再运行 `codex-peek --mcp`。这个进程是一个不提供任何工具的最小 MCP 服务器，负责打开窗口，并和窗口保持一个本地连接。Codex 退出时连接断开，窗口随之关闭。需要 PATH 里有 `node`（通过 npm 安装的 Codex 都有），不需要信任确认。
2. **Hook（备用方式）**：需要先执行一次 `/hooks` 信任插件的 hook。注意 Codex 要到**第一次提交提示词时**才触发 SessionStart，所以只用 hook 时，窗口要在第一次发送后才出现，届时会补上最近 10 分钟内的图片。

可以用环境变量 `CODEX_PEEK_ARGS` 传参数，例如 `--idle 120`；用 `CODEX_PEEK_BIN` 指定本地编译的二进制。

### 方式二：手动运行

```sh
npx codex-peek
# 或者自己编译
cargo build --release && ./target/release/codex-peek
```

参数：`--dir <目录>`、`--pattern <通配符>`（默认 `codex-clipboard-*.png`）、`--idle <秒>`、`--existing`、`--port <端口>`。

## 已知限制

- **编号可能对不上**：如果你在输入框里删了某个 `[Image #N]`，Codex 会重新编号，但 codex-peek 不知道。换下一条提示词前请点 **Clear**，或者用 `--idle`。
- 只能捕获**剪贴板粘贴**的图片。通过文件路径附加的图片（`codex -i`、拖入文件）和 **URL 图片**不会经过 `codex-clipboard-*.png`，所以不会显示。
- WSL 下需要加 `--pattern "tmp*.png"`，这样也可能匹配到其他程序的临时 PNG。
- 二进制还没有签名。
- 如果 Codex 崩溃、没有触发 SessionEnd，窗口会一直开着，点 ✕ 关闭即可。
- 已在 Linux/X11 上用 codex-cli 0.162.1 完整测试。MCP 方式需要 `node`；hook 方式需要先通过 `/hooks` 信任，而且第一次提交提示词后才会启动。

## 插件能做和不能做的事

能做：插件可以通过 `.codex-plugin/plugin.json` 的 `hooks` 字段提供 SessionStart/SessionEnd hook，hook 能拿到 `PLUGIN_ROOT`/`PLUGIN_DATA` 环境变量，Windows 有单独的 `commandWindows`。hook 进程脱离进程组后可以长期运行。
不能做：Codex 没有“图片粘贴”这个事件，插件也不能在 TUI 输入框里画图，所以只能用单独的窗口。插件 hook 要先经用户信任才会运行。SessionEnd 的超时被限制为 3 秒。插件不能按系统打包二进制，所以由 hook 脚本自动下载。

## 路线图

- 在 Codex 输入框里直接预览（kitty 图形协议或 sixel），参见 openai/codex issue [#29451](https://github.com/openai/codex/issues/29451) 和 [#35824](https://github.com/openai/codex/issues/35824)，计划给 Codex 提 PR。
- 删除占位符后重新编号（需要 Codex 提供相应事件）。
- macOS 签名和公证、Windows ARM64、Homebrew/winget/Scoop 安装。
- 用系统文件事件代替轮询。

## 许可证

MIT
