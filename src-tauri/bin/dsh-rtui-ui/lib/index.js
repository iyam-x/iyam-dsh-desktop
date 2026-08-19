// dsh-rtui-ui node half (host 侧): 注册持久化的自定义主题设置命名空间。
// 浏览器 client 半段(exports["./client"] → client.js)通过 ctx.settingsScope
// 读写该 namespace; 此处把 namespace 在 host 的 settings 服务里登记为可写、可持久化，
// 否则 settings.update 会因 "not registered" 被静默丢弃(对比度/强调色无反应)。
// 范式照搬官方 @deepseek-ai/dsh-client-ui-theme/lib/index.js。
import { settingsNamespace } from "@deepseek-ai/dsh-settings";
import z from "@deepseek-ai/schemastery";

export const name = "dsh-rtui-ui";

/** Host 与浏览器共享的命名空间; 须与 client.js 的 SETTINGS_NS 完全一致。 */
const RTUI_SETTINGS_NAMESPACE = "dsh-rtui";

/** 与 client.js 默认值保持一致的 schema; 字段可选, 浏览器侧再兜底。
 *  新增 preset/font/radius/density 控制增强 UI。 */
const RTUI_SETTINGS_SCHEMA = z.object({
  enabled: z.boolean().default(true),
  preset: z.string().default("graphite"),
  accent: z.string().default("#4D6BFE"),
  sidebarContrast: z.string().default("slightly"),
  font: z.string().default("system"),
  radius: z.string().default("medium"),
  density: z.string().default("comfortable"),
});

function apply(ctx) {
  ctx.inject(["settings"], (settingsCtx) => {
    settingsCtx.settings.register(settingsNamespace(RTUI_SETTINGS_NAMESPACE), RTUI_SETTINGS_SCHEMA);
  });
}

export { RTUI_SETTINGS_NAMESPACE, RTUI_SETTINGS_SCHEMA, apply };
