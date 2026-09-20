window.__ModuleLoader__.load({
	id: "@iyam/dsh-desktop-shell",
	factory: (require) => {
		var module = { exports: {} };
		var exports = module.exports;

		const name = "dsh-desktop-shell";
		const inject = ["sessions"];

		function apply(ctx) {
		// 顶栏让位：只把**主内容列**（center + rightbar）在 dsh 内部下移一个标题栏
		// 高度（30px，见桌面壳 src/index.css 的 --titlebar-h），使右上角窗口三键不压
		// dsh 头部工具区。左侧栏（sidebar）不参与——Win/Linux 的系统按钮只在右上角，
		// 侧栏保持贴窗口顶边（与改动前一致）。主内容上方那条 30px 由 frame 自身
		// 的 bg-base 绘制，与头部同色、无接缝，故无需壳层再画一条留白。
		// 选择器用 dsh 源码里的列类名子串（centerCol / rightbarCol），跨构建稳定；
		// 加 !important 以抵抗 dsh 自身样式的加载顺序，避免升级后被覆盖导致击穿。
		const isMac = /mac|iphone|ipad/i.test(navigator.userAgent);
		const style = document.createElement("style");
		style.id = "iyam-dsh-shell-css";
		style.textContent = `
[class*="centerCol"], [class*="rightbarCol"] {
  margin-top: 30px !important;
}
${isMac ? `
/* macOS 红绿灯在左上角，侧栏也要让开：侧栏 slot 是 display:contents（无盒模型，
   margin 不生效），故作用在其第一个可见子元素上。 */
[data-slot="sidebar"] > :first-child {
  margin-top: 10px;
}
` : ""}
`;
		document.head.appendChild(style);

			// 会话通知桥：监听会话 running 边沿（true→false = 对话完成），以及
			// pendingInteraction 出现（等待授权/审阅/回复），经 postMessage 转发给
			// 桌面壳，由宿主在窗口未聚焦时弹系统通知。
			// 方案与 dsh-rtui 验证过的 notify 插件一致：订阅 sessions.list 快照，
			// 首帧仅建基线，之后 running true→false 或 pending 出现才通知。
			const sessions = ctx.sessions;
			if (!sessions || !sessions.list) {
				// 运行时未提供 sessions 时不启用通知桥
				return;
			}

			const PENDING_LABEL = {
				approval: "等待你的授权",
				"plan-review": "等待你审阅计划",
				question: "等待你的回复",
			};

			function notify(title, body) {
				try {
					window.parent?.postMessage(
						{ source: "iyam-dsh-shell", type: "turn-end", reason: body, title },
						"*"
					);
				} catch (_e) {
					// 静默
				}
			}

			ctx.effect(() => {
				let prev = new Map();
				const unsub = sessions.list.subscribe(() => {
					let snap;
					try {
						snap = sessions.list.getSnapshot();
					} catch (_e) {
						return;
					}
					const ids = (snap && snap.ids) || [];
					const byId = (snap && snap.byId) || {};
					const cur = new Map();
					for (const id of ids) {
						const s = byId[id];
						if (!s) continue;
						cur.set(id, { running: !!s.running, pending: s.pendingInteraction || "" });
					}
					for (const [id, st] of cur) {
						const old = prev.get(id);
						prev.set(id, st);
						if (!old) continue; // 基线帧，不通知
						const display = (byId[id] && byId[id].displayTitle) ? byId[id].displayTitle : id;
						if (old.running && !st.running) {
							notify("DeepSeek Harness", `${display} 已完成回复`);
						} else if (!old.pending && st.pending) {
							const label = PENDING_LABEL[st.pending] || "等待你的输入";
							notify("DeepSeek Harness", `${display} — ${label}`);
						}
					}
					for (const id of prev.keys()) {
						if (!cur.has(id)) prev.delete(id);
					}
				});
				return unsub;
			}, "iyam-dsh-shell: sessions watcher");
		}

		exports.name = name;
		exports.inject = inject;
		exports.apply = apply;
		return module.exports;
	}
});

// ── 浏览器侧兜底：webview 可能把非 http(s) 的 window.open / 自定义 scheme 链接
//    / location 导航交给系统打开，触发「选择应用」对话框。统一拦截并记录。
//    原属已下线的 @iyam/dsh-file-handler（与文件预览无关，属浏览器侧通用加固），
//    迁到本插件；文件级执行，只要插件被加载即生效，不依赖任何运行时服务。 ──
const ALLOWED_URL = /^(https?:|data:|blob:|about:|javascript:|#)/i;
const origWindowOpen = window.open.bind(window);
window.open = function (url, ...rest) {
	const u = String(url ?? "");
	// 仅拦截带 scheme 且非白名单的地址（自定义 scheme）；相对地址放行，避免误伤 SPA 路由
	if (u && !ALLOWED_URL.test(u) && /^[a-z][a-z0-9+.-]*:/i.test(u)) {
		return null;
	}
	return origWindowOpen(url, ...rest);
};
document.addEventListener(
	"click",
	(e) => {
		const el = e.target && e.target.closest ? e.target.closest("a[href]") : null;
		if (!el) return;
		const href = el.getAttribute("href") || "";
		if (!href || ALLOWED_URL.test(href)) return;
		e.preventDefault();
		e.stopPropagation();
	},
	true
);
// 拦截 location 导航（location.href= / assign / replace）到自定义 scheme。
// 仅拦截"带 scheme 且非白名单"的地址（如 dsh://、app://、file://），避免 WebView2
// 把自定义 scheme 交给系统弹「选择应用」对话框。无 scheme 的相对地址（/path、./path）
// 是 SPA 内部路由，必须放行，否则"添加模型后返回列表"等相对跳转被误拦 → 界面不刷新。
const blockScheme = (url) => {
	const u = String(url || "");
	if (!u) return false;
	if (ALLOWED_URL.test(u)) return false;
	if (!/^[a-z][a-z0-9+.-]*:/i.test(u)) return false; // 无 scheme → 内部路由，放行
	return true;
};
const loc = window.Location?.prototype;
if (loc) {
	const hrefDesc = Object.getOwnPropertyDescriptor(loc, "href");
	if (hrefDesc && hrefDesc.set) {
		Object.defineProperty(loc, "href", {
			configurable: true,
			enumerable: true,
			get: hrefDesc.get,
			set(v) { if (!blockScheme(v)) hrefDesc.set.call(this, v); },
		});
	}
	for (const m of ["assign", "replace"]) {
		if (typeof loc[m] === "function") {
			const orig = loc[m];
			loc[m] = function (url) {
				if (blockScheme(url)) return undefined;
				return orig.apply(this, arguments);
			};
		}
	}
}
