# 更新日志

本项目所有用户可见变更均记录于此。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)。

## [0.2.15] - 2026-09-05

### 修复

- **界面加载后 `/api` 全部 401、WebSocket 连不上**：`/api`（含 `remote.mux` WS 升级）走的是
  另一条 cookie 认证链路（`requestRejection`），与 index 的 token 补丁不同源——跨源 iframe
  依旧没有 cookie。现扩展 web 认证补丁：Host/Origin fence（防 DNS rebinding）保留在前，
  其后对 **TCP 回环对端**（`req.socket.remoteAddress`，传输层事实、不可伪造）放行 cookie
  校验；局域网请求仍需认证，安全语义不变。
- **主题插件停用（`require dsh-client-runtime missed the module table`）**：dsh 0.1.2-rc.1
  移除了 `@deepseek-ai/dsh-client-runtime` 包，`defineStore` 改由内核 seed 模块
  `@deepseek-ai/dsh-client-store` 提供（`{ init, actions }` 契约不变）。`dsh-rtui-ui` 改为
  优先 require 新 seed 模块、旧版 dsh 回退旧 runtime；三个内置插件的 `dsh.client.inject`
  声明同步更新。

### 已知问题

- `dsh-file-handler`（文件内联预览）在 dsh 0.1.2-rc.1 上暂不可用：其拦截点
  `ctx.workspaces.openPath` / `api.host.openPath` 客户端服务已被上游移除（仅存 node 侧
  实现），需基于新 UI 的文件打开链路重新适配。插件按设计优雅降级（仅告警，不影响启动），
  旧版 dsh 上功能不受影响。

## [0.2.14] - 2026-09-05

### 修复

- **webview 中仍提示 `dsh web authentication required`（0.2.13 遗留）**：dsh 的签名 cookie
  带 `HttpOnly; SameSite=Strict`，而 app 的 DSH 界面运行在 `tauri://localhost` 顶层的跨源
  iframe 里——WebKit 将其按第三方 cookie 处理，cookie 既存不下也发不出，0.2.13 的「加载
  token URL 换 cookie」路径在 webview 中必然再次 401。现按项目既有上游补丁模式（同
  picker-owner 补丁）把 `dsh-client-connection` 的「token 换 cookie 后 303 重定向」改为
  「token 校验通过直接返回 index」，界面加载全程无需 cookie；token 仍是访问凭据（裸地址
  依旧 401），安全语义不变。补丁幂等应用于顶层与核心内嵌两份副本，升级 dsh 还原文件后
  每次启动自动重补；补丁刚生效时强制重启 DSH 让内存代码与磁盘一致。
  - 本机实测：补丁后带 token URL 直接 200 返回界面（含 `__DSH_BOOT__`），裸地址仍 401。

## [0.2.13] - 2026-09-05

### 修复

- **适配 dsh 0.1.2-rc.1 的 web 认证（launch token）**：该版本起 dsh web 全站认证，启动时
  stdout 打印的地址带 `?token=...`，首次访问以 token 换取签名 cookie，裸地址一律 401。
  此前 app 只解析端口、iframe 加载裸地址，升级后界面必然无法加载。现从 stdout 捕获完整
  带 token 的 URL（`dsh web: http://127.0.0.1:<port>/?token=...`，LAN 尾注不匹配），落盘
  `~/.dsh/.iyam-dsh.url` 并经 `dsh-port-ready` 事件传给前端，iframe 直接加载该地址；旧版
  dsh（无 token 裸 URL）行为不变。`start_dsh` 返回值由端口号改为完整 URL。
- **修复升级后「boot manifest batches must be an array」**：升级提升发生在启动早期，若旧
  dsh 进程仍在后台常驻（窗口关闭进托盘），app 会复用旧进程——内存里是旧版 dsh、磁盘已是
  新版，旧格式 boot manifest（无 `batches` 字段）被新版前端校验即报此错。现于 spawn 时把
  核心版本记入 `~/.dsh/.iyam-dsh.version`，启动时检测「运行中进程版本 ≠ 磁盘版本」（含记
  录缺失的升级遗留进程）即强制重启，保证内存与磁盘永远一致。
- 本机端到端实测：token URL → 303 换 cookie → 200 加载界面，manifest 含 `batches`(2)/
  `entries`(49)，裸 URL 401——新 dsh 完整可用。

## [0.2.12] - 2026-09-05

### 修复

- **dsh 升级后「整棵启动失败→回滚」的死循环**：第三方插件（如 dshmarket 1.20.0）静态 import 了
  dsh 0.1.2-rc.1 移除的内部导出（`installSettingsSection`），ESM 链接期即崩、拖垮整棵 DSH；
  旧自愈只识别 `Cannot find package`，识别不了这类链接错误，只能回滚并标记坏版本，用户被
  永久卡在旧版 dsh。现改为分层自愈，保证**始终可用最新版 dsh**：
  - **插件自动隔离（泛化）**：启动失败时从 stderr 解析肇事插件——`failed to import loader entry
    <id> (<pkg>)` 汇总行与栈里的 `node_modules/...` 路径（含 `.pnpm` 布局）——只禁用肇事者后重试；
    点不出具体插件时，最后一轮禁用全部第三方插件（`@deepseek-ai/*`、`@iyam/*` 核心除外），
    保证 DSH 必然能启动。`dshmarket` 不再被当作核心包保护。
  - **隔离记录与自动恢复**：被禁插件记入 `~/.dsh/.quarantine.json`（含当时 dsh 版本）；dsh
    版本变化后下次启动自动恢复重试（新版下插件可能已适配），仍不兼容则再次隔离；升级失败
    回滚时无条件全部恢复。系统通知告知用户被禁用了哪些插件。
  - **回滚后自动重启**：升级提升后启动失败并完成回滚时，app 自动用回滚后的版本重新拉起
    DSH，不再要求用户手动重开。
  - **内置插件自身加固**：`dsh-rtui-ui` host 侧改为动态 `import()` schemastery + `settings.register`
    全程守卫（`@iyam` 插件受隔离保护、无法被禁用，必须自身永不崩——此前静态 import 内部包
    是同类隐患）；client 侧 `require` 失败降级为 no-op。
  - 修复隔离判定中「已安装」检查的路径错误（unix 全局布局为 `lib/node_modules`，市场插件在
    `profiles/web/node_modules`），避免误剥离正常插件。

### 变更

- **放开 dsh 自动更新上限**：移除 `DSH_MAX_UPDATE_VERSION` 兼容上限机制（`cap_to_compat` /
  `update_held` / `compat_max`），托管更新始终升到 registry 最新版——可用性由上述启动期自愈
  兜底，不再因官方接口调整把用户卡在旧版。`bad_version` 防护保留（仅拦「核心都起不来」的版本）。

## [0.2.11] - 2026-09-04

### 变更

- **放开 dsh 自动更新上限到 `0.1.2-rc.1`**：内置 `@iyam/*` 插件已适配 dsh 0.1.2-rc.1 的 settings API。
  - `dsh-rtui-ui`：`@deepseek-ai/dsh-settings` 在 0.1.2-rc.1 移除了 `settingsNamespace` 导出，
    `settings.register` 改为直接收 namespace 字符串（须小写连字符，如 `"dsh-rtui"`）。移除该 import
    并把 `register(settingsNamespace(NS), SCHEMA)` 改为 `register(NS, SCHEMA)`。
  - `dsh-shell-plugin` / `dsh-file-handler` 经核对仅 `inject` `@deepseek-ai/dsh-client-runtime`，
    不依赖被移除的导出，无需改动。
  - `DSH_MAX_UPDATE_VERSION` 由 `0.1.1-rc.2` 提到 `0.1.2-rc.1`。

### 修复

- **避免对已坏版本反复重试**：`maybe_auto_stage` 现在读取 `~/.dsh/.update.json` 的 `bad_version`，
  若该版本正是 registry 最新版则跳过自动备货，避免「备货→启动失败→回滚」每 24h 循环
  （典型场景：用户装有旧版 `dshmarket`，在 0.1.2-rc.1 上因 `installSettingsSection` 被移除而崩）。
  `bad_version` 绑定了记录时的 app 版本——本 app 升级（内置插件重新适配）后旧坏标记自动失效，
  允许换新 app 后重试已修复的升级。

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
