// dsh-file-handler node half (host 侧)：纯浏览器端插件的占位。
// 实际逻辑在 client.js（包装 workspaces.openPath，转发文件点击给桌面壳预览）。
export const name = "dsh-file-handler";

export function apply(ctx) {
  void ctx; // 无服务端逻辑
}
