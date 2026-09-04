# 更新日志

本项目所有用户可见变更均记录于此。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)。

## [0.2.10] - 2026-09-04

### 修复

- **CI 编译失败（类型不匹配）**：0.2.9 在 `stage_update` 内把 `target_version` 收敛为 `String`，但下游 `install_dsh_to_tmp` 形参为 `&str`，传值时未借引用导致 `expected &str, found String`。修正为传 `&target_version`（自动解引用强制转换）。

## [0.2.9] - 2026-09-04

### 修复

- **「检查更新」不再显示新版本（回归）**：0.2.8 把兼容上限直接塞进了 `latest_dsh_version()`，导致它被当成「最新版」返回给界面，于是 registry 上更高的 dsh 版本被隐藏、界面永远显示「已是最新」。现改为：版本查询返回 registry **真实最新版**（恢复可见性），兼容上限只作用于**实际安装**——`bootstrap_dsh` / `stage_update` / `trigger_dsh_update` 在装包前用 `cap_to_compat()` 收敛到 `0.1.1-rc.2`。`UpdateInfo` 新增 `update_held` / `compat_max`：当确有更新但超兼容上限时，界面提示「新版本已发布，自动更新已暂停（兼容上限 vX）」，不再提供「下载并更新」按钮，避免把破坏性 dsh 版本装进来。

## [0.2.8] - 2026-09-04

### 修复

- **「下载并更新」拉到 dsh 0.1.2-rc.1 后启动报 `DSH 启动超时（30s）`**：该 dsh 版本移除了内置插件依赖的内部导出（`settingsNamespace` / `installSettingsSection`），导致本 app 内置的 `@iyam/*` 插件启动即抛 `does not provide an export named ...`，整棵 DSH 起不来。现增加**兼容性上限**：托管自动更新只升到 `0.1.1-rc.2`（内置插件针对的 dsh API 版本），超过则收敛回退、绝不拉入破坏性变更；待内置插件随新 dsh API 重新验证/移植后再上调该常量（`downloader.rs` 的 `DSH_MAX_UPDATE_VERSION`）。
- **回滚后 `@iyam` 插件丢失、下次仍启动失败**：升级提升时把 `@iyam` 从备份目录「移动」回 home（`move_dir`），而 `move_dir` 在 rename 失败时会回退为「复制后删源」，吃掉 `.backup` 里的 `@iyam`；一旦新版本启动失败走 `rollback_after_failure`，还原出的 node_modules 闭包便不再含 `@iyam` → 下一次启动仍报插件加载失败。改为用「复制」还原 `@iyam`（保留备份），使回滚后 `@iyam` 仍在、升级成功时由 `clear_applying` 清理备份。

## [0.2.7] - 2026-09-04

### 修复

- **「下载并更新」重启后报 `Failed to load plugins / client-modules: HTML did not preload @deepseek-ai/dsh-client-modules/client.js`**：升级提升（promote）只把 dsh 核心包挪到正式目录，但 dsh 的大量 `@deepseek-ai/*` 兄弟包（dsh-client-modules、dsh-host-frontend-static 等）由 npm 以 `--prefix` 安装时 hoist 到 `node_modules/@deepseek-ai/` 顶层、与 dsh 平级（实测 150+ 个包都在该层级），仅随 `.staging` 被删。结果 home 顶层残留旧版 `@deepseek-ai/*`，与新核心版本错配 → boot manifest 与 client-modules 对不上 → 加载失败。现改为**整体把 staging 的 `node_modules` 原子提升到 home**（保留 app 托管的 `@iyam` 插件目录），dsh 核心 + 全部 `@deepseek-ai/*` 依赖一次替换、版本必然一致；回滚逻辑同步改为还原整棵 `node_modules` 备份。

## [0.2.6] - 2026-09-04

### 修复

- **「下载并更新」重启后报 `Failed to load plugins / client-modules: boot manifest batches must be an array`**：升级 dsh 核心到新版后，顶层 `~/.dsh/node_modules/@deepseek-ai/*`（内置 `@iyam` 插件实际解析来源）仍残留旧版，与新版 `dsh-client-modules` / `dsh-host-frontend-static` 等字段错配——旧版 host-frontend-static 生成的 boot manifest 缺 `batches` 字段，新版 client-modules 校验该字段即抛错。现把提升兜底 `hoist_nested_dsh_deps()` 改为「版本不一致即覆盖顶层」而非「顶层存在就跳过」，并改为**每次启动都校准一次**（版本一致则零开销跳过），确保顶层 `@deepseek-ai/*` 永远与当前 core 内嵌版本一致。

## [0.2.5] - 2026-09-04

### 修复

- **「下载并更新」重启后报启动超时、且没更新到新版**：升级提升（promote）在启动时把 dsh 核心包（约 260MB / 3 万文件）整树复制两遍（当前版→备份、备货版→正式），且是「先删后拷」非原子顺序。复制耗时数分钟会把启动卡死并触发超时→回滚；中途失败则留下半残目录，DSH 必然起不来。现改为 `rename`（同卷 O(1) 原子移动，实测 3000 文件 46ms），偶发文件锁未释放时短暂重试后再回退复制；启动成功即清理 `.backup`（约 260MB 不再常驻）。
- **更新过程无进度提示**：点「下载并更新」时实时透出后端 `dsh-install-progress` 的阶段与百分比到标题栏 toast。

### 增强

- **备货完成提供「立即重启」**：升级成功提示旁增加重启按钮，走 `app.restart()`（先触发退出清理杀掉旧 DSH，再拉起进程使新版本生效），不必再手动退出重开。

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
