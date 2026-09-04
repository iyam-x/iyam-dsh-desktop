# 更新日志

本项目所有用户可见变更均记录于此。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)。

## [0.2.4] - 2026-09-04

### 修复

- **「检查更新」必报「安装后入口仍不可用，可能镜像源均异常」**：备货阶段的自愈校验 `dsh_entry_runs()` 用 `managed_node(home)` 推导 node 路径，而备货时传入的 `home` 是 `~/.dsh/.staging`，node 实际装在 `~/.dsh/node/` 下，于是永远找不到 node、把装好的好包误判为坏包，还会无谓地换官方源重装一次。改为由调用方显式传入托管 node 路径。
- **Windows 上备货版本无法提升（升级永远不生效）**：`apply_staged_if_ready()` 把 staging 内的包路径硬编码为 `staging/lib/node_modules/...`（类 Unix 布局），而 Windows 的 npm 全局布局是 `staging/node_modules/...`，复制源不存在导致提升失败并静默回退到旧版本。改为与正式目录一样走 `dsh_node_modules()` 推导。

## [0.2.3] - 2026-08-26

### 变更

- **运行时改为按需下载（不再内置）**：DSH 内核与 Node 运行时不再打包进安装包，改为首次启动经 npmmirror → npm 官方镜像回退下载并部署到 `~/.dsh/`（全局布局）。安装包体积从约 460MB 降至约 14MB；首次启动需联网（约 1~2 分钟），之后秒级启动。
- **DSH 升级改为"备货"机制**：「检查更新」发现新版时后台装到 `~/.dsh/.staging`，下次启动提升（apply）到正式目录，失败自动回滚到上一可用版本；版本未变不重新下载。
- **安装目录变更**：由 `~/.iyam-dsh/` 改为与用户自行 `npm i -g` 一致的 `~/.dsh/`，便于复用已装标准插件；PID / 端口文件重命名为 `.iyam-dsh.pid` / `.iyam-dsh.port`。

### 修复

- **启动闪 cmd 窗口**：`detect_dsh_cli` 的 `where` 探测与 npm 安装子进程补 `CREATE_NO_WINDOW`，消除初始化时多个控制台窗口闪烁。
- **退出闪「加载失败」**：应用退出时先隐藏主窗口 / iframe 再杀 DSH 进程，并调整 `dsh-app-exiting` 事件顺序（先于 kill 发出），避免后端被杀瞬间 iframe 闪现 DSH 的「加载失败」错误页。
- **启动变慢 / 像反复初始化**：`--no-open` 能力探测结果缓存到 `~/.dsh/.no-open-supported`，后续启动跳过这次约 3s 的 node 探测，加快启动。
- **解压闪 cmd 窗口**：Node 归档解压调用 `tar.exe` 补 `CREATE_NO_WINDOW`，消除初始化下载 Node 时仍会弹出的控制台窗口。
- **清空 `~/.dsh` 后首启 30s 超时**：npm 全局安装未保证把 dsh 内部的 `@deepseek-ai/*` 依赖 hoist 到顶层，导致内置 `@iyam/*` 插件 `import '@deepseek-ai/dsh-settings'` 报 `ERR_MODULE_NOT_FOUND`、DSH 起不来。新增兜底：安装/升级提升后把 dsh 嵌套的 `@deepseek-ai/*` 提升到 `~/.dsh/node_modules/@deepseek-ai/` 顶层（仅补缺失），让插件可解析。

### 增强

- **插件市场改为询问安装**：启动完成后若 `dshmarket` 尚未安装，弹窗询问用户是否安装；点击「安装」才经 `dsh plugin --profile web add dshmarket` 安装，点击「暂不安装」则不安装、不影响功能。不再首启强制联网安装，无网络/不需要时不再拖慢启动。

## [0.2.1] - 2026-08-26

### 修复

- **目录选择对话框任务栏多出 node 图标**：`ensure_picker_owner_patch` 此前只改了顶层 `worker.cjs`，漏掉 npm 嵌套副本（运行时实际加载的那一份），导致对话框无 owner、单独占任务栏按钮。改为递归遍历 `DSH_HOME/node_modules` 下所有 `dsh-host-directory-picker-native/lib/worker.cjs` 一并打补丁，对话框以主窗口为 owner，不再弹 node 图标。

## [0.2.2] - 2026-08-26

### 修复

- **插件装坏导致 DSH 启动超时**：插件市场安装带自定义 bundle 约定的元包（如 `@linxin666/dsh-web-ui-all`）时，可能只写进 `profiles/web/package.json` 的 bundles、却没把包（及其成员插件）真正装进 `~/.dsh/node_modules`，DSH 启动加载即 `ERR_MODULE_NOT_FOUND` 崩/挂，壳只报笼统的「启动超时（30s）」。现改为启动自愈：30s 内没出端口时，解析本次启动新增的 stderr，自动把「声明了却没装」或「成员包缺失」的非核心插件从 profile 剥离并重启（最多 3 轮）；核心包（`@iyam/*`、`@deepseek-ai/*`、`dshmarket`）永不剥离。剥离成功后发 `dsh-plugins-auto-disabled` 事件（前端可弹通知告知用户禁用项），仍起不来则透出真实缺失包名而非笼统超时。

## [0.1.0] - 2026-08-20

### 修复

- **插件加载失败**：`dsh-rtui-ui` 的 `client.js` 注册 id 由短名 `"dsh-rtui-ui"` 修正为完整包名 `"@iyam/dsh-rtui-ui"`，修复安装后启动报 `Failed to load plugins`。
- **托盘「退出」失效**：修复 `DSH_CHILD` 锁与 `child.wait()` 相互等待的死锁（此前表现为应用不退、托盘无响应）；退出路径恢复正常。
- **退出后 node 残留**：托盘「退出」前先显式清理，并按 `dsh.pid` 递归终止 node 进程树，应用退出时 DSH 一并关闭。
- **通知点击不唤起窗口**：系统通知点击后窗口无法被唤起。根因为 `notify-rust` 的 `wait_for_response` 对「无按钮的纯点击」不会真正阻塞等待（`needs_response()` 为 false → 底层 `should_wait=false`），点击激活永远收不到。改为 macOS 直接用 `mac_notification_sys` 弹通知并 `wait_for_click(true)`：整条 toast 可点击、`send()` 真正阻塞至用户点击，再经主线程把主窗口还原/显示/聚焦。（`tauri-plugin-notification` 因丢弃激活句柄，未采用。）

### 变更

- **主题设置精简**：移除字体、圆角、密度三项设置，设置面板仅保留启用、主题预设、强调色、侧栏对比度。

### 增强

- **强调色驱动主按钮**：主按钮填充/悬停改用强调色，并按强调色亮度反算文字色保证对比度；同时修复一处拼错的 brand token。
- **键盘焦点环**：`focus-visible` 描边使用强调色，提升键盘导航可见性。
- **主题切换过渡**：颜色/背景/边框 0.18s 柔和过渡，避免瞬间跳变。
- **收起侧边栏**：收起时侧栏颜色与主题底色保持一致，并去掉右边框（macOS 下与红绿灯更协调）。

## [0.1.1] - 2026-08-20

### 新增

- **文件内联预览**：点击 DSH 会话中产出/编辑的图片、音视频、文本/代码文件时，在应用内直接预览（图片 lightbox / 音视频内嵌播放 / 文本代码带行号与语法高亮），不再调用系统默认程序；其他类型（压缩包、可执行文件等）仍走系统默认打开。
  - 新增内置插件 `@iyam/dsh-file-handler`：包装 `workspaces.openPath`（所有文件打开入口的汇聚点），把可预览文件点击经 postMessage 桥转发给桌面壳。
  - 宿主新增预览浮层（`read_text_file` / `read_file_data` 命令读文件，highlight.js 语法高亮）。
- **插件安装修复**：内置插件刷新移到 DSH「已运行早退」之前；检测到运行中的 DSH 早于当前构建（缺少新插件）时自动杀掉重启，确保升级后新插件生效。

### 增强

- **上下文用量环对比度**：对话框头部上下文用量环的「已用」弧线改用强调色，与未用轨道形成清晰对比（原调色板 label-tertiary 过浅导致几乎同色）。
