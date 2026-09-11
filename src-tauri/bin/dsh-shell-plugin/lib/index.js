// 服务端 no-op：仅作为 bundle 的 Cordis 插件入口存在，
// 实际逻辑全部在 client.js（布局 CSS + 会话通知桥 + 自定义 scheme 导航兜底）。
const name = "dsh-desktop-shell";
const inject = [];

function apply() {
  // no-op
}

export { name, inject, apply };
