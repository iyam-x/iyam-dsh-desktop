// dsh-rtui-ui node half (host 侧): 注册持久化的自定义主题设置命名空间。
// 浏览器 client 半段(exports["./client"] → client.js)通过 ctx.settingsScope
// 读写该 namespace; 此处把 namespace 在 host 的 settings 服务里登记为可写、可持久化，
// 否则 settings.update 会因 "not registered" 被静默丢弃(对比度/强调色无反应)。
//
// 可用性契约：本模块是 app 内置插件，被 `is_core_bundle` 保护、每次启动强制刷新，
// 一旦抛错将拖垮整棵 dsh 且无法被自动隔离。因此**禁止静态 import dsh 内部包**
// （dsh 升级常移除/改名内部导出，静态 import 在 ESM 链接期即抛错），一律动态
// import + 全程守卫：任何失败只降级为「主题设置不持久化」，绝不阻断 dsh 启动。
export const name = "dsh-rtui-ui";

/** Host 与浏览器共享的命名空间; 须与 client.js 的 SETTINGS_NS 完全一致。 */
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

function apply(ctx) {
  ctx.inject(["settings"], (settingsCtx) => {
    import("@deepseek-ai/schemastery")
      .then((mod) => {
        const z = mod.default ?? mod;
        const schema = z.object({
          enabled: z.boolean().default(RTUI_DEFAULTS.enabled),
          preset: z.string().default(RTUI_DEFAULTS.preset),
          accent: z.string().default(RTUI_DEFAULTS.accent),
          sidebarContrast: z.string().default(RTUI_DEFAULTS.sidebarContrast),
          font: z.string().default(RTUI_DEFAULTS.font),
          radius: z.string().default(RTUI_DEFAULTS.radius),
          density: z.string().default(RTUI_DEFAULTS.density),
        });
        try {
          // dsh 0.1.2-rc.1 起 register 直接收 namespace 字符串（旧版收
          // settingsNamespace(ns) 对象，该导出已随版本移除）。签名再变时
          // register 抛错会被下方捕获，仅失去持久化，不影响主题生效。
          settingsCtx.settings.register(RTUI_SETTINGS_NAMESPACE, schema);
        } catch (e) {
          console.warn("[iyam/dsh-rtui-ui] settings.register 失败，主题设置不持久化:", e);
        }
      })
      .catch((e) => {
        console.warn("[iyam/dsh-rtui-ui] schemastery 不可用，跳过设置注册:", e);
      });
  });
}

export { RTUI_SETTINGS_NAMESPACE, apply };
