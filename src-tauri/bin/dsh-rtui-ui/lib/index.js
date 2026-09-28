// dsh-rtui-ui node half (host 侧): 声明本插件的可持久化设置 schema。
// 浏览器 client 半段(exports["./client"] → client.js)通过 ctx.configForms.get(entryId)
// 读写该 schema 产生的表单。
//
// dsh 0.1.7-rc.2 起设置体系重建：旧的 `settingsCtx.settings.register(ns, schema)`
// 已移除，改由 cordis 插件 Config 驱动——插件导出 `Config`（schemastery schema），
// settings describe 按 profile entry id（cordis.patch.yml 的 insert.id = "dsh-rtui-ui"）
// serve 表单，字段必须 `.volatile()` 才会成为可编辑/可持久化字段（对照官方
// dsh-client-ui-theme 的做法）。
//
// 可用性契约：本模块是 app 内置插件，被 `is_core_bundle` 保护、每次启动强制刷新，
// 一旦抛错将拖垮整棵 dsh 且无法被自动隔离。schemastery 是 dsh-settings 自身的
// 硬依赖（官方 ui-theme 亦静态 import），不属「易被改名移除的 dsh 内部包」。
import z from "@deepseek-ai/schemastery";

export const name = "dsh-rtui-ui";

/** Host 与浏览器共享的历史命名空间; 须与 client.js 的 SETTINGS_NS 完全一致。 */
const RTUI_SETTINGS_NAMESPACE = "dsh-rtui";

/** 与 client.js 默认值保持一致的字段缺省值。 */
const RTUI_DEFAULTS = {
  enabled: true,
  preset: "graphite",
  accent: "#4D6BFE",
  sidebarContrast: "slightly",
  font: "system",
  radius: "medium",
  density: "comfortable",
};

/** 可持久化的用户偏好（.volatile() 字段才会出现在 settings 表单里）。 */
const Config = z.object({
  enabled: z.boolean().default(RTUI_DEFAULTS.enabled).volatile(),
  preset: z.string().default(RTUI_DEFAULTS.preset).volatile(),
  accent: z.string().default(RTUI_DEFAULTS.accent).volatile(),
  sidebarContrast: z.string().default(RTUI_DEFAULTS.sidebarContrast).volatile(),
  font: z.string().default(RTUI_DEFAULTS.font).volatile(),
  radius: z.string().default(RTUI_DEFAULTS.radius).volatile(),
  density: z.string().default(RTUI_DEFAULTS.density).volatile(),
});

function apply(ctx, config) {
  // 主题的实际应用全部在 client 半段（读 configForms 快照 + overrideTokens），
  // host 侧无运行时逻辑；Config 导出本身即完成设置登记。
  void ctx;
  void config;
}

export { RTUI_SETTINGS_NAMESPACE, Config, apply };
