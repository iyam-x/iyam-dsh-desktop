# 首次问答后弹出 macOS "Choose Application" 对话框

- 日期：2026-08-21
- 状态：**未解决 / 待继续排查**
- 环境：macOS（aarch64）

## 症状

DSH **首次**发起带**代码块**的问答、回答完成时，macOS 弹出一个 **"Choose Application"** 对话框，要求为名为 `use_default` 的应用/文件选择打开方式。普通回答不触发；后续问答不再触发（仅首次）。

## 触发特征（已确认）

- 仅在**首次问答**且回答含**代码块**时触发。
- 触发时 iframe 内**没有任何 `[fh]` 日志**（已覆盖 `workspaces.openPath`、`api.host.openPath/openTextFile`、`window.open`、`<a href>` 点击、`location` 导航）。
- DSH host（node）stderr 落盘为空。
- 系统日志 `log show` 里没有对应 `open`/LaunchServices 事件；`process == "open"` 未捕获到任何 `open` 进程。
- `defaults read com.apple.LaunchServices/... LSHandlers` 中**没有** `use_default`（默认浏览器是 Microsoft Edge）。

## 已排查并排除的路径

| 层 | 手段 | 结论 |
| --- | --- | --- |
| iframe JS | `dsh-file-handler` 包装 `workspaces.openPath`、`api.host.openPath/openTextFile`、`window.open`、`<a>` 点击、`location.href/assign/replace` | 触发时零日志 → 非这些通道 |
| DSH host | 包装 `workspaces.openPath`；`api.host` 兜底 | 无调用 → 非浏览器端 RPC |
| open CLI | `log stream` 监听 `process == "open"` | 未执行 → 非 `open` 命令 |
| webview 导航 | 主窗口改 Rust builder + `on_navigation` 拦截非 http(s) scheme | **未拦下** → 非 webview 转发（已回退该改造） |
| default-browser | `bundle-name`/`default-browser-id` osascript | LSHandlers 无 `use_default` → 排除 |

## 当前结论

触发源是 DSH 内部某个**直接调用 `NSWorkspace`/LaunchServices 的原生模块**（node 侧 koffi 之类），绕过了浏览器端、host RPC、open CLI、webview 导航等所有可拦截层。`use_default` 是运行时构造的值，DSH 源码/数据中均无此字符串。

## 下一步（待执行）

1. 用 `sudo fs_usage -w -f exec | grep -iE 'use_default|open|osascript|launch'` 在复现期间捕获，定位实际执行的命令/进程。
2. 或检查 `dsh-host-directory-picker-native` 等含原生代码（koffi）的包在首次会话时是否直接打开文件。
3. 定位后决定：修复 DSH 行为 / 配置默认处理应用 / 接受为 DSH 自身 bug 并绕开。

## 相关代码状态

- `dsh-file-handler` 已保留：无扩展名非已知文件不调系统 open（转给壳）、`api.host.openPath/openTextFile` 兜底包装、`[fh]` 诊断日志（供继续排查）。
- `process.rs` 已保留：DSH host stderr 落盘到 `~/.iyam-dsh/dsh-stderr.log`（供排查）。
- 主窗口 `on_navigation` 改造**已回退**（未生效且增加窗口创建风险）。
