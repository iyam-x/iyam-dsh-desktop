// 服务端 no-op：仅作为 bundle 的 Cordis 插件入口存在，
// 实际逻辑全部在 client.js（向 DSH web UI 注入平台布局 CSS）。
const name = "dsh-desktop-shell";
const inject = [];

function apply() {
  // no-op
}

export { name, inject, apply };
