# QuickSpark

QuickSpark 是一个使用 Rust 开发的轻量、原生 Windows 快捷启动器。它以单文件形式运行，空闲常驻内存通常只有约 10 MB，实际占用会随 Windows 版本和使用状态略有变化。

按下 `Alt + Space`，输入应用名、文件、网址、命令或问题，然后按 `Enter`。它不需要账号，不做遥测，也不会为了搜索应用而在后台扫描整块硬盘。

## 它能做什么

- 搜索桌面、开始菜单和 Windows/UWP 应用，支持中文拼音和首字母。
- 查找并切换当前窗口。
- 调用 Everything 快速搜索本机文件。
- 打开网址，或使用百度、必应、Google 搜索。
- 进行计算、单位换算、日期计算和进制转换。
- 通过文本文件添加自己的网页、目录和工具快捷指令。
- 连接任意 OpenAI 兼容接口进行 AI 问答和翻译。
- 可选使用 Tavily，让 AI 自动搜索并核查实时信息。
- 原生 Win32 单文件程序，支持托盘、亮暗主题、DPI 缩放和开机启动。

## 下载和使用

从 [Releases](../../releases/latest) 下载 `quickspark.exe`，放到准备长期保存的位置后双击运行。

默认快捷键是 `Alt + Space`。如果它被其他软件占用，QuickSpark 会尝试其他内置组合，也可以在设置中心切换。

程序免安装，但设置保存在：

```text
%APPDATA%\QuickSpark
```

移动 EXE 后，如果开机启动失效，在设置中心关闭再重新开启一次即可。

## 常用输入

| 输入 | 用途 | 示例 |
| --- | --- | --- |
| 直接输入 | 搜索应用 | `obsidian`、`wx` |
| `f` / `file` | Everything 文件搜索 | `f *.pdf` |
| `win` / `window` | 查找并切换窗口 | `win chrome` |
| `web` | 使用默认搜索引擎 | `web 北京天气` |
| `bd` / `bing` / `gg` | 指定搜索引擎 | `gg rust` |
| `ai` | AI 问答 | `ai Rust 所有权` |
| `tr` / `翻译` | AI 翻译 | `tr --to ja Good morning` |
| `=` | 计算表达式 | `= 12 * (8 + 3)` |
| `%%` | 单位换算 | `%% 10 km to mi` |
| `date` | 日期计算 | `date today + 30d` |
| `hex` / `bin` / `oct` / `dec` | 进制转换 | `hex 255` |
| `sys` / `系统` | 系统操作 | `sys lock` |
| `$` | 搜索 Windows 设置 | `$ 蓝牙` |
| `;` | 自定义快捷指令 | `; docs` |
| `settings` / `设置` | 打开设置中心 | `设置 主题` |
| `?` / `help` | 查看内置说明 | `? AI` |

## 常用按键

| 按键 | 作用 |
| --- | --- |
| `Enter` | 执行当前结果 |
| `↑` / `↓` | 选择结果 |
| `Tab` | 打开动作菜单 |
| `Ctrl + Enter` | 以管理员身份运行桌面程序 |
| `Ctrl + Shift + E` | 打开文件所在位置 |
| `Ctrl + C` | 复制路径、网址或结果 |
| `Esc` | 隐藏窗口；AI 生成中用于取消 |
| `F1` | 打开内置说明 |

关闭搜索窗口只会隐藏 QuickSpark。需要彻底退出时，请使用托盘右键菜单中的“退出”。

## 可选功能

### Everything 文件搜索

使用文件搜索前，需要安装并运行 Everything 1.4.1 或更高版本。QuickSpark 直接使用 Everything 已有索引，不会自己遍历磁盘。

```text
f 报告
f *.pdf
file quickspark
```

### AI 问答和翻译

输入 `ai settings` 打开 AI 设置。你需要提供一个 OpenAI 兼容的 Chat Completions 地址和模型名：

```text
api_url=https://api.example.com/v1/chat/completions
model=example-model
max_tokens=8192
timeout_seconds=60
```

常用入口：

```text
ai key              设置或清除 AI API Key
ai tavily key       设置或清除 Tavily API Key
ai --web 问题       强制联网搜索
ai --no-web 问题    不联网，直接回答
```

API Key 不会写入配置文件，而是保存在 Windows 凭据管理器中。远程 AI 接口必须使用 HTTPS；只有本机回环地址允许 HTTP。

翻译默认把中文译成英语、非中文译成简体中文，也可以指定目标语言：

```text
tr 你好，世界
tr --to en 你好，世界
tr --to ja Good morning
tr ->ko Hello
```

支持英语、简体中文、日语和韩语。

### 自定义快捷指令

输入 `; edit` 打开配置文件，每行格式如下：

```text
别名 | 显示名称 | 目标
```

例如：

```text
docs | 打开文档目录 | %USERPROFILE%\Documents
gh | 搜索 GitHub | https://github.com/search?q={query}
```

使用时输入 `; docs` 或 `; gh quick launcher`。

## 隐私

- 没有账号、遥测、云同步、搜索历史或聊天历史。
- 应用搜索、计算和系统入口完全在本机完成。
- 只有执行网页搜索、AI 问答或翻译时才会向对应服务发送内容。
- AI Key 和 Tavily Key 分开保存在 Windows 凭据管理器中。
- QuickSpark 不会把 Key 写进普通配置文件，也不会跟随 HTTP 重定向转发凭据。

## 从源码构建

需要 Windows、Rust 1.95.0、MSVC 工具链和 Windows SDK。

```powershell
git clone <仓库地址>
cd quick-launcher
cargo build --release
.\target\release\quickspark.exe
```

常用检查：

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --release
```

## 发布新版本

项目已配置 GitHub Actions。版本标签与 `Cargo.toml` 中的版本一致时，会自动在 Windows runner 上测试和构建，并把 EXE 与 SHA-256 校验文件上传到 GitHub Release。

当前版本为 `0.1.3`，发布命令示例：

```powershell
git tag v0.1.3
git push origin main
git push origin v0.1.3
```

工作流见 [.github/workflows/release.yml](.github/workflows/release.yml)。

## 项目定位

QuickSpark 想做的是一个启动快、行为清楚、放在后台不打扰你的 Windows 小工具，而不是另一个带账号、插件市场和聊天记录的完整平台。
