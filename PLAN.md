# iyam-dsh-desktop — 完整实施计划

## 一、产品定位

将 `@deepseek-ai/dsh` 封装为跨平台原生桌面应用。用户无需任何 Node.js/npm 知识，双击图标即可使用完整 DSH 功能。

- **App 本体**：~14MB（Tauri 壳 + Rust 二进制 + 前端）
- **内置 DSH**：~333MB（完整 `@deepseek-ai/dsh` 包，打包进 app Resources）
- **内置 Node**：~116MB/平台（Node 24 LTS，随 app bundle 分发，零系统依赖）
- **总计**：~460MB（一次打包，永久使用，无需网络、无需系统安装 Node.js）
- **首次启动**：从内置资源复制到 `~/.iyam-dsh/`，瞬间完成
- **后续启动**：直接复用本地安装，秒级启动
- **内核不变**：`@deepseek-ai/dsh` 源码零修改，通过 App 内「检查更新」独立升级

---

## 二、技术选型

| 维度 | 选择 | 理由 |
|------|------|------|
| 壳框架 | **Tauri v2** | ~15MB 包体，复用系统 WebView，原生托盘/菜单 |
| 前端 | TypeScript + React + Vite | 快速开发，与 DSH 生态一致 |
| 后端 | Rust (Tauri) | 进程管理稳定，交叉编译友好 |
| Node 运行时 | **内置 Node 24 LTS**（`scripts/fetch-node.mjs` 按平台下载） | 零系统依赖，满足原生模块 ABI（node-pty napi-v9 / sharp ≥20.9） |
| DSH 调用 | Rust 直接 spawn `node lib/bin.js web` | 不经过 shell 脚本，Windows 天然支持 |
| UI 集成 | **WebviewWindow 直连** | 无 iframe 边框，完整原生窗口体验 |
| 分发 | macOS `.app/.dmg`，Windows `.exe`，Linux `.AppImage` | |

---

## 三、目录结构

```
iyam-dsh-desktop/
├── package.json                    # Tauri v2 + Vite + React 项目
├── scripts/
│   └── fetch-node.mjs              # 按平台下载内置 Node 运行时
├── src/
│   ├── main.tsx                    # React 入口
│   ├── App.tsx                     # 主页面（Loading / Error / Ready 三状态）
│   └── index.css                   # 全局样式
├── src-tauri/
│   ├── bin/
│   │   ├── dsh-package/            # 内置 DSH 完整包（333MB）
│   │   └── node/<平台>/            # 内置 Node 运行时（~116MB/平台）
│   ├── Cargo.toml                  # Rust 依赖
│   ├── src/
│   │   ├── main.rs                 # Tauri 入口：tray + menu + auto-start
│   │   ├── process.rs              # 用内置 node 管理 DSH 进程生命周期
│   │   ├── installer.rs            # 检测 + 部署内置 DSH 到 ~/.iyam-dsh/
│   │   └── updater.rs              # 版本检查与提示更新
│   ├── tauri.conf.json             # 窗口配置、bundle、icon
│   ├── capabilities/               # 权限策略
│   │   └── default.capabilities.json
│   ├── capabilities/installer.capabilities.json
│   ├── icons/                      # 各平台图标
│   └── build.rs                    # 构建脚本（打包 dsh-package + node 进 Resources）
└── PLAN.md
```

---

## 四、核心模块设计

### 4.1 `installer.rs` — 安装器

**职责**：确保 `~/.iyam-dsh/` 下有可用的 dsh 安装（零网络、零系统依赖）

```rust
// 1. DSH_HOME = dirs::home_dir() + "/.iyam-dsh"
// 2. 检查 <DSH_HOME>/bin/dsh(.cmd) 是否存在且可执行
//    ├─ 存在且有效 → Ok(true) 无需安装
//    └─ 不存在 → 触发安装流程
// 3. 安装流程（纯本地复制，无网络）：
//    a. 定位内置资源：app.path().resource_dir() → bin/dsh-package + bin/node/<平台>/node
//    b. 复制内置 dsh-package 到 <DSH_HOME>/
//    c. 生成 bin/dsh（unix sh）或 bin/dsh.cmd（windows），指向内置 node 绝对路径
// 4. 安装失败 → 返回 Error("内置资源不完整，请重新安装应用")
```

**关键实现细节**：
- 内置 Node 由 `scripts/fetch-node.mjs` 下载，构建期随 bundle 分发
- `node_target()` 用编译期 cfg 映射当前平台 → 资源目录名
- 不读取系统 node、不执行 npm install

### 4.2 `process.rs` — 进程管理

```rust
// spawn_dsh_web(dsh_home: &Path) -> Result<(Child, u16), Error>
//   1. 定位内置 node：app.path().resource_dir() → bin/node/<平台>/node(.exe)
//   2. 构造命令: <node> <dsh_home>/lib/bin.js web --port 0
//      （不经 shell，无 wrapper，Windows 天然支持）
//   3. 设置 env:
//      - DSH_HOME = dsh_home
//   4. 捕获 stdout（端口日志）与 stderr
//   5. 启动子进程
//   6. 轮询 stdout，匹配正则 "dsh web: http://127.0.0.1:(\\d+)"
//   7. 超时 30s 未获取到端口 → Error("DSH 启动超时")
//   8. 返回 (Child, port)

// kill(child: &mut Child) -> Result<(), Error>
//   unix: SIGTERM → 等待 5s → 若未退出则 SIGKILL
//   windows: taskkill /F /PID

// is_running(child: &Child) -> bool
//   child.state() != Blocked
```

**关键实现细节**：
- 启动前先 `get_install_status`，未安装才执行安装，避免每次启动重复拷贝 333MB
- PID 文件 `~/.iyam-dsh/dsh.pid` + 端口文件 `dsh.port`，二次启动复用

### 4.3 `updater.rs` — 版本检查

```rust
// check_for_update(installed_version: &str) -> Result<Option<String>, Error>
//   1. GET https://registry.npmjs.org/@deepseek-ai/dsh/latest
//   2. 解析 JSON 取 `dist-tags.latest`
//   3. 比较 semver: 远端 > 本地 → Some(新版本号)
//   4. 否则 → None

// prompt_update(new_version: &str) -> bool
//   弹系统对话框询问用户是否更新
//   用户确认 → 重新运行 installer（下载新版本）→ 重启 App
```

### 4.4 `main.rs` — Tauri 入口

```rust
fn main() {
    // 1. 初始化 Tauri builder
    // 2. 注册命令:
    //    - "install_dsh" → 调用 installer.rs
    //    - "start_dsh"   → 调用 process.rs spawn
    //    - "stop_dsh"    → 调用 process.rs kill
    //    - "check_update"→ 调用 updater.rs
    // 3. 创建系统托盘（最小化到托盘、退出）
    // 4. 创建主窗口（WebviewWindow）
    // 5. 启动顺序：
    //    a. 先检查安装状态（installer）
    //    b. 安装完成后启动 DSH 进程（process）
    //    c. 获取端口后打开 WebviewWindow
}
```

---

## 五、前端状态机

```
App.tsx 三状态流转：

  [INIT] ──install_dsh──→ [INSTALLING] ──spawn──→ [READY]
    │                        │                      │
    │  (已有安装，跳过)       │  (失败)              │  (关闭 App)
    ▼                        ▼                      ▼
  [READY]              [ERROR] ←── retry ────────── [STOPPED]
                                         ▲
                                         │
                                   (网络/安装问题)
```

- **INIT**：启动时调用 `install_dsh` 命令，若已有安装则立即返回 READY
- **INSTALLING**：显示进度条 + "正在安装 DeepSeek Harness..." 文案
- **ERROR**：显示错误信息 + 重试按钮
- **READY**：隐藏所有状态 UI，WebviewWindow 已加载 DSH 界面

---

## 六、`tauri.conf.json` 关键配置

```json
{
  "productName": "iyam-dsh",
  "version": "0.1.0",
  "identifier": "ai.iyam.dsh",
  "build": {
    "devUrl": "http://localhost:1420",
    "beforeDevCommand": "pnpm dev",
    "beforeBuildCommand": "pnpm build"
  },
  "app": {
    "windows": [],
    "withGlobalTauri": true
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": ["icons/32x32.png", "icons/128x128.png", ...],
    "resources": ["bin/dsh-package", "bin/node"],
    "macOS": {
      "entitlements": null,
      "frameworks": [],
      "minimumSystemVersion": "10.15"
    },
    "windows": {
      "wix": {
        "language": "zh-CN"
      }
    }
  },
  "security": {
    "csp": null
  }
}
```

---

## 七、构建与分发

### 开发调试
```bash
pnpm tauri dev          # 启动开发服务器 + Tauri 窗口
```

### 生产构建
```bash
pnpm tauri build
# 产物位置：
#   macOS:  src-tauri/target/release/bundle/macos/iyam-dsh.app
#           src-tauri/target/release/bundle/dmg/iyam-dsh_<版本>_aarch64.dmg
#   Windows: src-tauri/target/release/bundle/nsis/iyam-dsh_<版本>_x64-setup.exe
#   Linux:   src-tauri/target/release/bundle/appimage/iyam-dsh_<版本>_amd64.AppImage
```

### Release 打包发布操作步骤

> Tauri 不支持跨平台交叉打包，**每个平台必须在对应系统上构建**（macOS 包只能在 macOS 上出，Windows 包只能在 Windows 上出）。

#### 0. 发布前检查清单

- [ ] `pnpm fetch:node --target <当前平台>` 已执行，`src-tauri/bin/node/<平台>/node` 存在
- [ ] 内置 DSH 包 `src-tauri/bin/dsh-package/` 存在（.gitignore 排除，需手动放置）
- [ ] `pnpm tauri dev` 完整流程通过（安装 → 启动 → DSH Web UI 可访问）
- [ ] `~/.iyam-dsh` 有测试产生的配置时，确认不影响发布验证（可临时 `rm -rf ~/.iyam-dsh`）

#### 1. 版本号统一（三处保持一致）

```bash
# 同步修改 version 为同一值（如 0.2.0）：
#   - package.json      → "version"
#   - src-tauri/Cargo.toml → [package] version
#   - src-tauri/tauri.conf.json → "version"
# 修改后提交一个 chore commit，如：chore: bump version to 0.2.0
```

#### 2. 构建 macOS 包（在 macOS 上执行）

```bash
pnpm fetch:node --target darwin-arm64   # Apple Silicon
pnpm tauri build
# 产物：bundle/dmg/iyam-dsh_<版本>_aarch64.dmg
```

Intel Mac 构建时改用 `--target darwin-x64`，产物为 `_x64.dmg`。

#### 3. 构建 Windows 包（在 Windows 上执行）

```powershell
pnpm install
pnpm fetch:node --target win32-x64
pnpm tauri build
# 产物：bundle/nsis/iyam-dsh_<版本>_x64-setup.exe
```

#### 4. 构建 Linux 包（在 Linux 上执行）

```bash
pnpm install
pnpm fetch:node --target linux-x64
pnpm tauri build
# 产物：bundle/appimage/iyam-dsh_<版本>_amd64.AppImage
# 若缺 AppImage 依赖，先安装：libfuse2、libwebkit2gtk-4.1-dev 等
```

#### 5. macOS 签名与公证（可选但建议）

未签名版本用户会看到"无法验证开发者"提示（需右键打开）。正式分发建议：

```bash
# 1) Apple Developer 账号配置环境变量
export APPLE_ID="your@email.com"
export APPLE_APP_SPECIFIC_PASSWORD="xxxx-xxxx-xxxx-xxxx"
export APPLE_TEAM_ID="XXXXXXXXXX"
export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"

# 2) tauri.conf.json 的 bundle.macOS 增加：
#   "signingIdentity": "Developer ID Application: Your Name (TEAMID)"
#   "dmg": { "sign": true }, "app": { "hardenedRuntime": true }

# 3) 重新构建即自动签名 + 公证
pnpm tauri build
```

> 注意：内置 node 二进制为 Node.js 官方签名，公证一般可正常通过；如被拒，需在公证前对 node 二进制做 `codesign --force --deep` 重新签名。

#### 6. 上传发布到 Gitee Releases

1. 访问 https://gitee.com/scrm/iyam-dsh-desktop/releases → 「新建发行版」
2. 填写：版本号（Tag 与 `tauri.conf.json` 一致）、Release 标题（如 `v0.2.0`）、更新说明
3. 上传产物：
   - macOS: `iyam-dsh_<版本>_aarch64.dmg`（+ `_x64.dmg` 若有）
   - Windows: `iyam-dsh_<版本>_x64-setup.exe`
   - Linux: `iyam-dsh_<版本>_amd64.AppImage`
4. 若为预发布，勾选「预发布」；正式版发布时取消勾选

#### 7. 发布后验证

- [ ] 从 Releases 下载各平台安装包，在干净环境安装
- [ ] 首次启动：断网安装验证（内置部署、无需网络）
- [ ] 验证 `~/.iyam-dsh/bin/dsh --version` 可手动执行
- [ ] README「安装包下载」表格中的文件名与实际产物一致

### CI/CD（可选）
GitHub Actions / Gitee Go 按上述步骤自动构建并上传 Releases；需在 CI 中执行对应平台的 `pnpm fetch:node --target`。

---

## 八、兼容性清单

| DSH 功能 | 桌面 App 中的行为 |
|----------|-----------------|
| `dsh plugin add <pkg>` | 完全可用（DSH 进程内有 terminal tool） |
| `dsh plugin remove <pkg>` | 完全可用 |
| `dsh --profile web --help` | App 固定使用 web profile |
| `DSH_HOME` 环境变量 | App 固定为 `~/.iyam-dsh/` |
| dsh 内核升级 | App 菜单「检查更新」或手动 `npm update -g @deepseek-ai/dsh` |
| Agent Presets | 读取 `~/.iyam-dsh/.agent-presets/`，完整保留 |
| Cordis 插件系统 | 完整保留，无感知 |

---

## 九、实施步骤（建议顺序）

1. **创建 Tauri 项目脚手架**（`pnpm create tauri-app`）
2. **下载内置 Node**（`scripts/fetch-node.mjs` + `pnpm fetch:node`）
3. **实现 `installer.rs`**：内置资源定位 + 本地复制 + 跨平台启动脚本
4. **实现 `process.rs`**：内置 node 直启 + 端口监听 + kill
5. **实现 `main.rs`**：Tauri 入口 + 托盘 + 命令注册
6. **实现前端 `App.tsx`**：状态机 UI
7. **集成联调**：`pnpm tauri dev` 完整流程测试
8. **打包各平台**：`pnpm tauri build`
9. **macOS 代码签名**（发布前）

---

## 十、风险评估

| 风险 | 影响 | 应对 |
|------|------|------|
| 内置 Node 与 DSH 原生模块 ABI 不匹配 | 无法加载 node-pty/sharp | 选定 LTS 后先验证原生模块加载；升级 Node 前跑兼容性测试 |
| macOS Gatekeeper 拦截未签名 App | 无法打开 | 发布时进行代码签名；提供公证指引 |
| Windows WebView2 版本过旧 | 渲染异常 | 启动时检测 WebView2 版本，不足则引导安装 |
| 包体过大（~460MB） | 分发/下载慢 | 单平台构建，仅内置当前平台 node |
| DSH 新版本的 breaking change | 兼容性问题 | 保持对最新版 dsh 的测试覆盖 |
