# 首次问答后弹出 macOS "Choose Application" 对话框

- 日期：2026-08-21
- 状态：**已修复（待 mac 真机验证）**
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

## 当前结论（已推翻旧推测）

旧记录推测「DSH 内部某原生模块直接调 NSWorkspace/LaunchServices」——**不成立**：bundle 内无 NSWorkspace 代码，koffi 仅用于 Windows COM（目录选择器/ACL），mac 目录选择器走 `osascript choose folder`。

**实际根因是 wry 0.55.1 的两处行为 + WKWebView 兜底**：

1. **响应级漏洞**：`wry/src/wkwebview/navigation.rs` 的 `decidePolicyForNavigationResponse` 在响应 MIME 无法被 WebKit 渲染（如 `application/octet-stream`）且**未设置下载 handler 时，直接 `Allow`**。WKWebView 收到无法渲染的响应后会交给 LaunchServices → 弹 "Choose Application"。`use_default` 是 WebKit 侧兜底名，DSH 源码里不存在。
2. **导航动作全放行**：未设置 `on_navigation` 时 wry 对任意 URL 导航一律 `Allow`。
3. **window.open 兜底**：未设置 new-window handler 时 `createWebViewWith` 返回 nil，WebKit 把 URL 交给 `NSWorkspace`。

三者都**不产生 JS 日志、不调 `open` CLI、不经过 `on_navigation`**，与「零日志 / 无 open 进程 / on_navigation 未拦下」的观测完全吻合。

## 修复（已实施）

主窗口从 `tauri.conf.json` 配置式改为 `src-tauri/src/main.rs` setup 手动构建，挂两层原生拦截：

1. **`on_download`**（主修复）：设置下载 handler 后，wry 对「无法渲染的 MIME 响应」改走 `.download` 交给下载流程保存到系统下载目录，**不再弹系统对话框**。
2. **`on_navigation`**（防御）：只放行壳页面来源（dev `localhost:1420` / prod `tauri://`）+ DSH 回环 `127.0.0.1` + IPC + `about:blank`，其余（`file://`、自定义 scheme、外部站点）一律 Cancel。

窗口参数与旧配置一致（Windows 无边框、macOS 透明 Overlay 标题栏）。

## 验证

- Windows：`tauri build --debug --no-bundle` 编译通过，冒烟启动窗口正常创建。
- macOS：**待真机验证**——首次带代码块问答后不应再弹对话框；日志 `[webview-download]` 可确认被改走下载的内容。
- 注意：tauri CLI 会按平台自动增删 Cargo.toml 的 `macos-private-api` feature（Windows 构建时移除、macOS 构建时加回），属 CLI 既有行为。
- 顺带修复：DSH 启动失败——`process.rs` 此前传了 DSH 0.1.0-rc.7 不支持的 `--no-open`（commander 报 `unknown option '--no-open'`），已改为「bundle 为最新（≥rc.8）时始终传 `--no-open`」（rc.8 起该选项受支持且默认会开浏览器）；并让前端错误提示按平台显示 `dsh` / `dsh.cmd`。
- DSH 升级到 rc.8 后重新打包的附加修复：
  - `scripts/fetch-dsh.mjs`：Windows 上 `execFileSync("npm")` 无法执行 `npm.cmd`（加 `shell: true`）；新增 `flattenNodeModules` 压平 `--install-strategy nested` 产生的超深 node_modules（makensis 读不了 >260 字符路径，rc.8 曾达 476），同版本冗余副本删除/软链解引用。
  - `src-tauri/build.rs`：`copy_dir_all` 改为跟随软链复制 + 真实路径循环保护（bundle 内 `node_modules/@deepseek-ai/dsh` 自引用软链与 profiles 软链此前导致 `fs::copy` 报拒绝访问）。

## 排查过程要点

- `use_default` 在 DSH 全部源码/前端 dist/依赖中不存在（仅 dsh-file-handler 注释里提到）。
- 三个自定义插件（file-handler/shell/rtui-ui）只做拦截/布局/通知/主题，无原生打开调用，**不是触发源**。
- `dsh-host-frontend-static` 对未知扩展名静态资源一律回 `application/octet-stream`，是潜在「无法渲染」响应来源。

## 额外加固（后续迭代）

`dsh-file-handler` client.js 做了两层增强：

1. **更宽泛的 `isNonFilePath` 判断**：用 `[\\/][\\/]` 检测路径分隔符（覆盖 Windows `\`），防止绝对路径被误判为"非文件"。
2. **多层兜底包装**：除 `workspaces.api?.host` 外，还遍历 `workspaces.api` 自身属性，找到所有含 `openPath`/`openTextFile` 的方法并包装，防止通过不同实例引用绕过拦截。用 `__fhPatched` 标记避免重复包装。

## 相关代码状态

- `dsh-file-handler` 已保留：无扩展名非已知文件不调系统 open（转给壳）、`api.host.openPath/openTextFile` 兜底包装、`[fh]` 诊断日志。
- `process.rs` 已保留：DSH host stderr 落盘到 `~/.iyam-dsh/dsh-stderr.log`。
