window.__ModuleLoader__.load({
	id: "@iyam/dsh-desktop-shell",
	factory: (require) => {
		var module = { exports: {} };
		var exports = module.exports;

		const name = "dsh-desktop-shell";
		const inject = [];

		function apply() {
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
		}

		exports.name = name;
		exports.inject = inject;
		exports.apply = apply;
		return module.exports;
	}
});
