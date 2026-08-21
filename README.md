# iyam-dsh-desktop

DeepSeek Harness（DSH）的跨平台原生桌面客户端。内置完整 **DSH 内核**与 **Node.js 运行时**，**无需联网、无需预先安装 Node.js** 即可开箱即用。

[![Build](https://github.com/OWNER/iyam-dsh-desktop/actions/workflows/build.yml/badge.svg)](https://github.com/OWNER/iyam-dsh-desktop/actions/workflows/build.yml)

> 将上方 `OWNER` 替换为本仓库在 GitHub 的实际所有者（个人名或组织名）。

## 功能特性

- **零网络依赖**：DSH 内核与 Node 运行时随安装包内置，首次启动自动部署到本地 `~/.iyam-dsh/`，之后秒开。
- **跨平台**：macOS（Intel / Apple Silicon）、Windows 10/11、主流 Linux 发行版。
- **完整 DSH 生态兼容**：`dsh plugin add/remove`、Agent Presets、Cordis 插件系统、内核升级等全部可用。
- **系统通知点击唤起**：对话完成后的系统通知，点击即可把应用窗口带到前台。

## 安装包下载

发布版安装包在 [Releases](https://github.com/OWNER/iyam-dsh-desktop/releases) 页面，按操作系统选择：

| 平台 | 安装包 | 说明 |
| --- | --- | --- |
| macOS（Apple Silicon） | `iyam-dsh_<版本>_aarch64.dmg` | 拖入「应用程序」即可 |
| macOS（Intel） | `iyam-dsh_<版本>_x64.dmg` | 同上 |
| Windows 10/11（x64） | `iyam-dsh_<版本>_x64-setup.exe` | 双击安装 |
| Linux（x64） | `iyam-dsh_<版本>_amd64.AppImage` / `iyam-dsh_<版本>_amd64.deb` | AppImage `chmod +x` 后运行；deb 用 `apt install` |

### 系统要求

- macOS 10.15+ / Windows 10 1803+ / 主流 Linux 发行版
- 磁盘剩余空间 ≥ 1GB（应用包约 460MB，解压部署后共约 800MB）
- **不需要** 预先安装 Node.js 或 npm

### 首次启动

1. 双击应用，会短暂显示「正在安装 DeepSeek Harness…」（从内置资源部署到 `~/.iyam-dsh/`，约 5~10 秒，**无需网络**）。
2. 部署完成后自动启动 DSH 并加载 Web UI。
3. 后续启动直接复用本地安装，秒开。

### 常见问题

- **macOS 提示"无法验证开发者"**：当前为未签名/未公证版本。首次打开请右键点击图标 → 「打开」；正式发布后会提供签名公证版本。
- **Windows 提示 SmartScreen**：点击「更多信息」→「仍要运行」。
- **卸载**：删除应用 + 删除 `~/.iyam-dsh/` 目录即可完全清除。

## 从源码构建（开发者）

### 前置依赖

- [Node.js](https://nodejs.org/) 20+
- [Rust](https://www.rust-lang.org/) 稳定版工具链
- [pnpm](https://pnpm.io/) 9
- 平台原生依赖：
  - **Linux**：`libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libgtk-3-dev build-essential pkg-config libssl-dev python3`
  - **macOS**：Xcode Command Line Tools（`xcode-select --install`）
  - **Windows**：Visual Studio 2022 含「使用 C++ 的桌面开发」

### 步骤

```bash
pnpm install                 # 安装前端依赖
node scripts/fetch-node.mjs  # 下载内置 Node 运行时（按当前平台）
pnpm tauri:dev               # 开发模式（热重载）
pnpm tauri:build             # 生产构建（构建前自动拉取内置 DSH 内核）
```

> `pnpm tauri:build` 会先执行 `scripts/fetch-dsh.mjs`，从 npm registry 拉取最新 `@deepseek-ai/dsh`（含原生模块）并打包进应用资源。**首次构建需联网**；离线时会复用已捆绑的版本并给出告警。

构建产物位于 `src-tauri/target/release/bundle/` 下对应平台的目录。

## 持续集成 / 自动构建

仓库已配置 GitHub Actions（`.github/workflows/build.yml`）：推送 `v*` 标签或手动触发 `workflow_dispatch` 时，会在 **macOS（aarch64 + x64）、Windows、Linux** 四个运行器上并行构建，并将各平台安装包作为 Assets 上传到一个**草稿 Release**（`releaseDraft: true`），供你检查后发布。

- 首次构建会下载 DSH 内核与 Node 运行时，单次构建耗时较长，属正常现象。
- 如需对 macOS 产物做签名/公证，在项目 Secrets 中填入 `APPLE_CERTIFICATE`、`APPLE_CERTIFICATE_PASSWORD`、`APPLE_SIGNING_IDENTITY`、`APPLE_ID`、`APPLE_PASSWORD`、`APPLE_TEAM_ID` 即可自动生效（留空则产出未签名包）。

## 工作原理

```
iyam-dsh-desktop (Tauri v2)
├── Rust 后端：进程管理、内置 DSH 解压到 ~/.iyam-dsh/
├── React 前端：状态 UI（Loading / Ready / Error）
├── 内置 DSH 包（~333MB，零网络依赖）
├── 内置 Node 运行时（~116MB/平台，零系统依赖）
└── 嵌入 DSH Web UI：WebView 直连 http://127.0.0.1:<port>
```

1. **内置 DSH + Node**：`src-tauri/bin/dsh-package/`（DSH 内核）+ `src-tauri/bin/node/<平台>/node`，随 app bundle 分发。
2. **首次启动**：检测 `~/.iyam-dsh/` 是否存在，不存在则从内置资源复制并生成指向内置 node 的启动脚本。
3. **进程管理**：用内置 node spawn `lib/bin.js web --port 0`，监听 stdout 获取端口，通过 Tauri Event 通知前端。
4. **UI 渲染**：前端收到端口后用 `<iframe src="http://127.0.0.1:<port>">` 加载 DSH Web UI。

零系统依赖：不读取系统 node、不读取系统全局安装的 dsh。手动终端使用可通过 `~/.iyam-dsh/bin/dsh`（Windows 为 `dsh.cmd`）。

## 项目结构

```
iyam-dsh-desktop/
├── package.json                  # Tauri + Vite 项目配置
├── scripts/
│   ├── fetch-dsh.mjs             # 构建前拉取内置 DSH 内核（npm registry）
│   └── fetch-node.mjs            # 下载内置 Node 运行时（按平台）
├── src/                          # React 前端
├── src-tauri/
│   ├── bin/dsh-package/          # 内置 DSH 完整包（.gitignore，构建时拉取）
│   ├── bin/node/                 # 内置 Node 运行时（.gitignore，构建时拉取）
│   ├── bin/dsh-{shell,rtui-ui,file-handler}/  # 内置插件
│   ├── src/{main,installer,process,notify}.rs
│   ├── tauri.conf.json           # 窗口、bundle、安全配置
│   ├── capabilities/             # 权限策略
│   └── build.rs                  # 将 dsh-package + node 打包进 app Resources
├── .github/workflows/build.yml   # 跨平台自动构建
└── PLAN.md                       # 完整方案文档
```

## 与 DSH 生态兼容

| 功能 | 行为 |
| --- | --- |
| `dsh plugin add <pkg>` / `dsh plugin remove <pkg>` | ✅ 完整可用 |
| Agent Presets | ✅ 读取 `~/.iyam-dsh/.agent-presets/` |
| Cordis 插件系统 | ✅ 完整保留 |
| DSH 内核升级 | 菜单「检查更新」或 `npm update -g @deepseek-ai/dsh` |

## 风险提示

- **代码签名 / 公证**：macOS / Windows 发布前建议进行平台签名与公证，否则用户首次打开会有系统安全警告。
- **Windows WebView2**：确保用户系统已安装 WebView2 Runtime（Windows 10/11 默认已安装）。
- **应用体积**：约 460MB（含 DSH 内核 + Node 运行时），但首次启动无需网络、无需系统依赖。
- **内置 Node 升级**：修改 `scripts/fetch-node.mjs` 的 `DSH_NODE_VERSION` 后重新执行 `node scripts/fetch-node.mjs`。

## 许可证

本项目以 [MIT 许可证](./LICENSE) 开源。
