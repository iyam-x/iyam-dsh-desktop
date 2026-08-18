# iyam-dsh-desktop

DeepSeek Harness 的跨平台原生桌面应用。内置完整 DSH 内核 **和 Node.js 运行时**，无需网络连接、无需安装 Node.js 即可首次使用。

## 架构

```
iyam-dsh-desktop (Tauri v2)
├── Rust 后端：进程管理、内置 DSH 解压到 ~/.iyam-dsh/
├── React 前端：状态 UI（Loading / Ready / Error）
├── 内置 DSH 包（333MB，零网络依赖）
├── 内置 Node 运行时（~116MB/平台，零系统依赖）
└── 嵌入 DSH Web UI：WebView 直连 http://127.0.0.1:<port>

DSH 内核（@deepseek-ai/dsh）与 Node.js 完整内置，首次启动自动部署到 ~/.iyam-dsh/
启动时直接用内置 node 运行 lib/bin.js，不依赖系统 node / 系统 dsh。
```

## 技术栈

- **壳框架**：Tauri v2（Rust 后端 + WebView 原生窗口）
- **前端**：React 18 + TypeScript + Vite
- **包管理**：pnpm

## 安装包下载

> 面向终端用户。下载即用，无需安装 Node.js，无需网络。

### 下载地址

发布版安装包位于 [Releases 页面](https://gitee.com/scrm/iyam-dsh-desktop/releases)，按操作系统选择对应文件：

| 平台 | 安装包 | 说明 |
| --- | --- | --- |
| macOS（Apple Silicon） | `iyam-dsh_<版本>_aarch64.dmg` | 拖入「应用程序」即可 |
| macOS（Intel） | `iyam-dsh_<版本>_x64.dmg` | 同上 |
| Windows 10/11（x64） | `iyam-dsh_<版本>_x64-setup.exe` | 双击安装 |
| Linux（x64） | `iyam-dsh_<版本>_amd64.AppImage` | `chmod +x` 后运行 |

### 系统要求

- macOS 10.15+ / Windows 10 1803+ / 主流 Linux 发行版
- 磁盘剩余空间 ≥ 1GB（应用包 ~460MB，解压部署后共约 800MB）
- **不需要** 预先安装 Node.js 或 npm

### 首次启动

1. 双击应用，会短暂显示「正在安装 DeepSeek Harness…」（从内置资源部署到 `~/.iyam-dsh/`，约 5~10 秒，**无需网络**）
2. 安装完成后自动启动 DSH 并加载 Web UI
3. 后续启动秒开，直接复用本地安装

### 常见问题

- **macOS 提示"无法验证开发者"**：应用尚未签名公证。首次打开请右键点击图标 → 「打开」；正式发布后会提供签名版本
- **Windows 提示 SmartScreen**：点击「更多信息」→「仍要运行」
- **卸载**：删除应用 + 删除 `~/.iyam-dsh/` 目录即可完全清除

## 快速开始

### 开发调试

```bash
pnpm install          # 安装依赖
pnpm fetch:node       # 下载内置 Node 运行时（仅需一次）
pnpm tauri dev        # 启动开发模式（自动使用 src-tauri/bin/dsh-package）
```

### 生产构建

```bash
pnpm tauri build      # 构建各平台原生安装包
```

构建产物位于 `src-tauri/target/release/bundle/macos/iyam-dsh.app`。

## 工作原理

1. **内置 DSH + Node**：`src-tauri/bin/dsh-package/`（333MB DSH）+ `src-tauri/bin/node/<平台>/node`（~116MB，Node 24 LTS），均随 app bundle 分发
2. **首次启动**：App 检测 `~/.iyam-dsh/` 是否存在
  
  - 不存在 → 从内置资源复制 DSH 到 `~/.iyam-dsh/`，创建指向内置 node 的启动脚本
  - 存在 → 直接复用
3. **进程管理**：直接用内置 node spawn `lib/bin.js web --port 0`，监听 stdout 获取端口，通过 Tauri Event 通知前端
4. **UI 渲染**：前端收到端口后，用 `<iframe src="http://127.0.0.1:<port>">` 加载 DSH Web UI

零系统依赖：不读取系统 node、不读取系统 npm 全局安装的 dsh。手动终端使用可通过 `~/.iyam-dsh/bin/dsh`（Windows 为 `dsh.cmd`）。

## 项目结构

```
iyam-dsh-desktop/
├── package.json                 # Tauri + Vite 项目配置
├── scripts/fetch-node.mjs       # 下载内置 Node 运行时（按平台）
├── src/
│   ├── main.tsx                 # React 入口
│   ├── App.tsx                  # 主页面（状态机）
│   └── index.css                # 全局样式
├── src-tauri/
│   ├── bin/dsh-package/         # 内置 DSH 完整包（333MB，.gitignore）
│   ├── bin/node/                # 内置 Node 运行时（~116MB/平台，.gitignore）
│   ├── src/
│   │   ├── main.rs              # Tauri 入口 + 命令注册
│   │   ├── installer.rs         # 检测并部署内置 DSH + 生成启动脚本
│   │   ├── process.rs           # 用内置 node 管理 DSH 进程生命周期
│   │   └── updater.rs           # 版本检查（可选）
│   ├── tauri.conf.json          # 窗口、bundle、安全配置
│   ├── capabilities/            # 权限策略
│   ├── build.rs                 # 构建脚本：将 dsh-package + node 打包进 app Resources
│   └── icons/                   # 应用图标（基于 logo.png）
└── PLAN.md                      # 完整方案文档
```

## 与 DSH 生态兼容

| 功能 | 行为 |
| --- | --- |
| dsh plugin add <pkg> | ✅ 完整可用 |
| dsh plugin remove <pkg> | ✅ 完整可用 |
| Agent Presets | ✅ 读取 ~/.iyam-dsh/.agent-presets/ |
| Cordis 插件系统 | ✅ 完整保留 |
| DSH 内核升级 | App 菜单「检查更新」或手动 npm update -g @deepseek-ai/dsh |

## 风险提示

- **macOS 代码签名**：发布前需进行 Apple 代码签名和公证（内置 node 二进制为 Node.js 官方签名，通常可正常通过）
- **Windows WebView2**：需确保用户系统已安装 WebView2 Runtime（Windows 10/11 默认已安装）
- **App Bundle 大小**：~460MB（含 333MB DSH + 116MB Node），但首次启动无需网络、无需系统依赖
- **DMG 打包**：需要 macOS SDK 签名工具，CI 环境需配置
- **内置 Node 升级**：修改 `scripts/fetch-node.mjs` 的 `DSH_NODE_VERSION` 后重新执行 `pnpm fetch:node`

​