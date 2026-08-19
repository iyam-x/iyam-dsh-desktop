window.__ModuleLoader__.load({
	id: "@iyam/dsh-desktop-shell",
	factory: (require) => {
		var module = { exports: {} };
		var exports = module.exports;

		const name = "dsh-desktop-shell";
		const inject = [];

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

			// 原生通知桥：agent 回合结束（completed / max-tokens / error）时通知宿主，
			// 由宿主（Tauri 父页面）在窗口未聚焦时弹系统通知。
			// 服务不可用时静默降级，不影响布局注入。
			try {
				ctx.conversationEvents.register({
					kind: "dsh-desktop-turn-end",
					match(event) {
						if (event.type === "turn/end") {
							return { id: String(event.data.turn), role: "start" };
						}
						return null;
					},
					start(_context, match) {
						const reason = match.event.data.reason.kind;
						if (reason === "completed" || reason === "max-tokens" || reason === "error") {
							try {
								window.parent?.postMessage(
									{ source: "iyam-dsh-shell", type: "turn-end", reason },
									"*"
								);
							} catch (_e) {
								// 静默
							}
						}
						return {};
					},
					update() {
						return {};
					},
				});
			} catch (_e) {
				// 静默：运行时未提供 conversationEvents 时不启用通知桥
			}
		}

		exports.name = name;
		exports.inject = inject;
		exports.apply = apply;
		return module.exports;
	}
});
