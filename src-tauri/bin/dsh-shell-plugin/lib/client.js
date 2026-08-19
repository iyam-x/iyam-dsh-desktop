window.__ModuleLoader__.load({
	id: "@iyam/dsh-desktop-shell",
	factory: (require) => {
		var module = { exports: {} };
		var exports = module.exports;

		const name = "dsh-desktop-shell";
		const inject = ["sessions"];

		function apply(ctx) {
			// 注入平台布局 CSS：避让原生窗口控件
			const isMac = /mac|iphone|ipad/i.test(navigator.userAgent);
			const style = document.createElement("style");
			style.id = "iyam-dsh-shell-css";
			style.textContent = `
/* macOS：左侧栏整体下移，避开左上角红绿灯。
   侧栏 slot 元素是 display:contents（无盒模型，margin 不生效），
   因此作用在其第一个可见子元素上。 */
[data-slot="sidebar"] > :first-child {
  margin-top: ${isMac ? "10px" : "0"};
}
/* Windows：顶部右侧控件左移，避开右上角窗口按钮（46px × 3 = 138px） */
${isMac ? "" : `
[class*="_titleRow"] {
  padding-right: 138px;
}
`}
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
