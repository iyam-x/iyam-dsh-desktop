// iyam-dsh 主题插件（移植自 dsh-rtui 的 dsh-rtui-ui，并增强）。
// 运行在 DSH web UI iframe(http://127.0.0.1:<port>/) 内，使用 window.__ModuleLoader__
// 老式客户端插件 API(与 @iyam/dsh-desktop-shell 同源)。
//
// 阶段1: 通过 ctx.theme.overrideTokens 注入主题 token(亮/暗成对)，并注入结构性 CSS。
// 阶段2(增强): 4 套预设调色板(石墨灰/午夜蓝/纸白/Sepia) + 强调色驱动主按钮/焦点环。
// 窗口 chrome(标题栏)在本地 webview 实现，此处不注入。
//
// 强调色走干净 token 钩子，稳健。

window.__ModuleLoader__.load({
  id: "@iyam/dsh-rtui-ui",
  factory: (require) => {
    const module = { exports: {} };
    const exports = module.exports;
    const react = require("react");
    const h = react.createElement;
    const runtime = require("@deepseek-ai/dsh-client-runtime/client");
    const { defineStore } = runtime;

    // ── 调色板：每套预设给出 light/dark 语义色 ──
    // 字段: bg/layer1-3 背景分层, label/label2/label3/caption/dimmed 文字,
    // borderRGB 边框 RGB(配合 alpha 生成层级边框), sidebar 侧栏基色,
    // brand 前景/主按钮色, btnHover 主按钮悬停, fg 主按钮上的文字色(亮=白,暗=背景)。
    const PALETTES = {
      graphite: {
        light: { bg: "#F7F7F8", layer1: "#FFFFFF", layer2: "#F7F7F8", layer3: "#F2F2F3",
          label: "#1A1A1D", label2: "#6F6F76", label3: "#9A9AA0", caption: "#B0B0B5", dimmed: "#C0C0C5",
          borderRGB: "0,0,0", sidebar: "#EFF0F2", brand: "#1A1A1D", btnHover: "#3A3A3D", fg: "#FFFFFF" },
        dark: { bg: "#161618", layer1: "#1E1E20", layer2: "#26262A", layer3: "#2C2C30",
          label: "#E9E9EC", label2: "#9B9BA1", label3: "#78787F", caption: "#66666C", dimmed: "#55555B",
          borderRGB: "255,255,255", sidebar: "#121214", brand: "#E9E9EC", btnHover: "#FFFFFF", fg: "#161618" },
      },
      midnight: {
        light: { bg: "#EEF1F8", layer1: "#FFFFFF", layer2: "#E9EDF6", layer3: "#E0E5F0",
          label: "#1B2233", label2: "#5A678A", label3: "#8A97B5", caption: "#A6B0C8", dimmed: "#B7C0D6",
          borderRGB: "20,40,90", sidebar: "#E4E9F4", brand: "#1B2A4A", btnHover: "#2E4068", fg: "#FFFFFF" },
        dark: { bg: "#0E1320", layer1: "#161C2C", layer2: "#1E2638", layer3: "#263044",
          label: "#DCE4F5", label2: "#8A97B5", label3: "#6B779A", caption: "#586589", dimmed: "#485277",
          borderRGB: "150,170,220", sidebar: "#0A0E18", brand: "#DCE4F5", btnHover: "#FFFFFF", fg: "#0E1320" },
      },
      paper: {
        light: { bg: "#FBFAF7", layer1: "#FFFFFF", layer2: "#F6F4EF", layer3: "#EFEDE6",
          label: "#2A2622", label2: "#7A736A", label3: "#A39B90", caption: "#BBB4A8", dimmed: "#C9C3B8",
          borderRGB: "60,50,40", sidebar: "#F4F1EA", brand: "#2A2622", btnHover: "#4A433B", fg: "#FFFFFF" },
        dark: { bg: "#1B1916", layer1: "#24211D", layer2: "#2C2823", layer3: "#342F29",
          label: "#F1ECE4", label2: "#A89E92", label3: "#847B6F", caption: "#6E665C", dimmed: "#5C554C",
          borderRGB: "240,230,215", sidebar: "#151311", brand: "#F1ECE4", btnHover: "#FFFFFF", fg: "#1B1916" },
      },
      sepia: {
        light: { bg: "#F5ECDD", layer1: "#FBF4E8", layer2: "#F0E6D2", layer3: "#E8DCC4",
          label: "#4A3B27", label2: "#8A7350", label3: "#B09770", caption: "#C2AC88", dimmed: "#CDB993",
          borderRGB: "90,65,30", sidebar: "#EFE3CD", brand: "#4A3B27", btnHover: "#6B5538", fg: "#FFFFFF" },
        dark: { bg: "#2A2218", layer1: "#332A1E", layer2: "#3D3223", layer3: "#483B29",
          label: "#EAD9BE", label2: "#B59B76", label3: "#8F7A58", caption: "#766347", dimmed: "#635438",
          borderRGB: "235,215,180", sidebar: "#221C14", brand: "#EAD9BE", btnHover: "#FBF4E8", fg: "#2A2218" },
      },
    };

    const PRESETS = [
      { id: "graphite", label: "石墨灰" },
      { id: "midnight", label: "午夜蓝" },
      { id: "paper", label: "纸白" },
      { id: "sepia", label: "Sepia" },
    ];
    const ACCENTS = [
      { id: "#4D6BFE", label: "品牌蓝" },
      { id: "#30A46C", label: "青绿" },
      { id: "#F5A623", label: "琥珀" },
      { id: "#E5484D", label: "品红" },
      { id: "#8B5CF6", label: "靛紫" },
    ];
    const CONTRASTS = [
      { id: "same", label: "与主内容一致" },
      { id: "slightly", label: "略深" },
      { id: "deeper", label: "更深" },
    ];

    function withAlpha(hex, a) {
      const m = hex.replace("#", "");
      const r = parseInt(m.slice(0, 2), 16);
      const g = parseInt(m.slice(2, 4), 16);
      const b = parseInt(m.slice(4, 6), 16);
      return `rgba(${r}, ${g}, ${b}, ${a})`;
    }
    function darken(hex, amt) {
      const m = hex.replace("#", "");
      const r = Math.round(parseInt(m.slice(0, 2), 16) * (1 - amt));
      const g = Math.round(parseInt(m.slice(2, 4), 16) * (1 - amt));
      const b = Math.round(parseInt(m.slice(4, 6), 16) * (1 - amt));
      return `rgb(${r}, ${g}, ${b})`;
    }
    function lighten(hex, amt) {
      const m = hex.replace("#", "");
      const ch = (c) => Math.round(parseInt(c, 16) + (255 - parseInt(c, 16)) * amt);
      return `rgb(${ch(m.slice(0, 2))}, ${ch(m.slice(2, 4))}, ${ch(m.slice(4, 6))})`;
    }
    function relLuminance(hex) {
      const m = hex.replace("#", "");
      const f = (c) => { const s = parseInt(c, 16) / 255; return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4); };
      return 0.2126 * f(m.slice(0, 2)) + 0.7152 * f(m.slice(2, 4)) + 0.0722 * f(m.slice(4, 6));
    }
    // 主按钮用 accent 填充时，按 accent 亮度反算按钮文字色，保证对比度。
    function accentFg(accent, isLight) {
      const lum = relLuminance(accent);
      return isLight ? (lum > 0.55 ? "#1A1A1D" : "#FFFFFF") : (lum > 0.5 ? "#161618" : "#FFFFFF");
    }
    function sidebarFill(p, contrast, isLight) {
      const bg = isLight ? p.light.bg : p.dark.bg;
      if (contrast === "same") return bg;
      if (contrast === "deeper") return darken(bg, isLight ? 0.06 : 0.12);
      return isLight ? p.light.sidebar : p.dark.sidebar; // slightly（默认）
    }

    // 把调色板 + accent + 对比度 展开成完整 token 表(沿用官方语义 token 命名)。
    function expandTokens(presetId, { accent, sidebarContrast }) {
      const p = PALETTES[presetId] || PALETTES.graphite;
      const L = p.light, D = p.dark;
      const sbL = sidebarFill(p, sidebarContrast, true);
      const sbD = sidebarFill(p, sidebarContrast, false);
      const acTintL = withAlpha(accent, 0.12), acTintD = withAlpha(accent, 0.18);
      const acHoverL = withAlpha(accent, 0.10), acHoverD = withAlpha(accent, 0.14);
      const acBtnFill = accent;
      const acBtnHoverL = darken(accent, 0.10), acBtnHoverD = lighten(accent, 0.12);
      const acBtnFgL = accentFg(accent, true), acBtnFgD = accentFg(accent, false);
      return {
        // 背景分层
        "--dsw-alias-bg-base": { light: L.bg, dark: D.bg },
        "--dsw-alias-bg-layer-1": { light: L.layer1, dark: D.layer1 },
        "--dsw-alias-bg-layer-2": { light: L.layer2, dark: D.layer2 },
        "--dsw-alias-bg-layer-3": { light: L.layer3, dark: D.layer3 },
        "--dsw-alias-bg-overlay": { light: L.layer1, dark: D.layer1 },
        "--dsw-alias-bg-multi-select": { light: L.layer3, dark: D.layer3 },
        "--dsw-alias-bg-module-platform": { light: L.layer3, dark: D.layer3 },
        "--dsw-alias-bg-skeleton": { light: "rgba(0,0,0,0.04)", dark: "rgba(255,255,255,0.08)" },
        // 边框
        "--dsw-alias-border-l1": { light: withAlpha(L.borderRGB, 0.06), dark: withAlpha(D.borderRGB, 0.06) },
        "--dsw-alias-border-l2": { light: withAlpha(L.borderRGB, 0.10), dark: withAlpha(D.borderRGB, 0.10) },
        "--dsw-alias-border-l2-darkmode-thin": { light: withAlpha(L.borderRGB, 0.10), dark: withAlpha(D.borderRGB, 0.06) },
        "--dsw-alias-border-l3": { light: withAlpha(L.borderRGB, 0.14), dark: withAlpha(D.borderRGB, 0.14) },
        "--dsw-alias-border-l4": { light: withAlpha(L.borderRGB, 0.18), dark: withAlpha(D.borderRGB, 0.20) },
        "--dsw-alias-border-inverted": { light: withAlpha(L.borderRGB, 0), dark: withAlpha(D.borderRGB, 0.06) },
        "--dsw-alias-border-inverted2": { light: withAlpha(L.borderRGB, 0), dark: withAlpha(D.borderRGB, 0.08) },
        // 文字
        "--dsw-alias-label-primary": { light: L.label, dark: D.label },
        "--dsw-alias-label-secondary": { light: L.label2, dark: D.label2 },
        "--dsw-alias-label-tertiary": { light: L.label3, dark: D.label3 },
        "--dsw-alias-label-caption": { light: L.caption, dark: D.caption },
        "--dsw-alias-label-dimmed": { light: L.dimmed, dark: D.dimmed },
        "--dsw-alias-label-primary-dimmed": { light: L.label2, dark: D.label },
        "--dsw-alias-label-primary-foreground": { light: L.fg, dark: D.fg },
        "--dsw-alias-label-primary-inverted": { light: L.fg, dark: D.fg },
        // 品牌
        "--dsw-alias-brand-primary": { light: L.brand, dark: D.brand },
        "--dsw-alias-brand-text": { light: L.brand, dark: D.brand },
        // 主按钮文字色按 accent 亮度反算（保证对比度）
        "--dsw-alias-brand-primary-invert": { light: acBtnFgL, dark: acBtnFgD },
        // 按钮: 强调色驱动主按钮填充与悬停
        "--dsw-alias-button-primary-fill": { light: acBtnFill, dark: acBtnFill },
        "--dsw-alias-button-primary-hover": { light: acBtnHoverL, dark: acBtnHoverD },
        "--dsw-alias-button-primary-dimmed": { light: L.layer3, dark: D.layer3 },
        "--dsw-alias-button-contrast-fill": { light: L.brand, dark: D.brand },
        "--dsw-alias-button-elevated-fill": { light: L.layer1, dark: D.layer1 },
        "--dsw-alias-button-floating-fill": { light: L.layer1, dark: D.layer1 },
        "--dsw-alias-button-floating-hover": { light: L.layer3, dark: D.layer3 },
        "--dsw-alias-button-ghost-active-border": { light: withAlpha(L.borderRGB, 0.22), dark: withAlpha(D.borderRGB, 0.30) },
        "--dsw-alias-button-ghost-active-fill": { light: L.layer3, dark: D.layer3 },
        "--dsw-alias-button-ghost-active-hover": { light: L.layer3, dark: D.layer3 },
        "--dsw-alias-button-tool-bar-fill": { light: "rgba(128,128,128,0.30)", dark: "rgba(255,255,255,0.18)" },
        "--dsw-alias-button-tool-bar-hover": { light: "rgba(128,128,128,0.40)", dark: "rgba(255,255,255,0.26)" },
        "--dsw-alias-button-tool-bar-fill-invisible": { light: "rgba(0,0,0,0.12)", dark: "rgba(255,255,255,0.10)" },
        // 交互
        "--dsw-alias-interactive-bg-hover": { light: "rgba(0,0,0,0.05)", dark: "rgba(255,255,255,0.06)" },
        "--dsw-alias-interactive-bg-active": { light: "rgba(0,0,0,0.09)", dark: "rgba(255,255,255,0.10)" },
        "--dsw-alias-interactive-bg-hover-accent": { light: acHoverL, dark: acHoverD },
        "--dsw-alias-interactive-bg-hover-solid": { light: L.layer3, dark: D.layer3 },
        "--dsw-alias-interactive-bg-hover-danger": { light: "rgba(236,19,19,0.05)", dark: "rgba(236,19,19,0.12)" },
        // 状态（与 DESIGN token 对齐，business 跟随 accent）
        "--dsw-alias-state-success-primary": { light: "#30A46C", dark: "#30A46C" },
        "--dsw-alias-state-success-secondary": { light: "#46A758", dark: "#46A758" },
        "--dsw-alias-state-error-primary": { light: "#E5484D", dark: "#E5484D" },
        "--dsw-alias-state-error-secondary": { light: "#F2555A", dark: "#F2555A" },
        "--dsw-alias-state-warn-primary": { light: "#F5A623", dark: "#F5A623" },
        "--dsw-alias-state-warn-secondary": { light: "#F5B14A", dark: "#F5B14A" },
        "--dsw-alias-state-warn-label": { light: "#B45309", dark: "#F5A623" },
        "--dsw-alias-state-business-primary": { light: accent, dark: accent },
        "--dsw-alias-state-business-tertiary": { light: acTintL, dark: acTintD },
        // 侧栏
        "--dsw-specific-sidebar-fill": { light: sbL, dark: sbD },
        "--dsw-specific-sidebar-nav-item-active": { light: L.layer3, dark: D.layer3 },
        "--dsw-specific-sidebar-nav-item-hover": { light: L.layer2, dark: D.layer2 },
        "--dsw-specific-sidebar-nav-item-active-accent": { light: acTintL, dark: acTintD },
        // markdown / 代码块
        "--dsw-alias-markdown-code-block": { light: L.layer3, dark: D.bg },
        "--dsw-alias-markdown-code-block-banner": { light: L.layer3, dark: D.layer3 },
        "--dsw-alias-markdown-inline-code": { light: L.layer3, dark: D.layer3 },
        "--dsw-alias-markdown-code-segment-selected": { light: L.layer1, dark: D.layer1 },
        "--dsw-alias-markdown-code-segment-unselected": { light: L.layer3, dark: D.layer3 },
        "--dsw-alias-markdown-citation": { light: L.layer3, dark: D.layer3 },
        "--dsw-alias-markdown-tag": { light: L.layer3, dark: D.layer3 },
        "--dsw-alias-markdown-placeholder": { light: L.layer3, dark: D.layer3 },
        // 浮层 / 气泡
        "--dsw-specific-bubble": { light: L.layer1, dark: D.layer1 },
        "--dsw-specific-bubble-highlight": { light: L.layer3, dark: D.layer3 },
        "--dsw-specific-menu": { light: L.layer1, dark: D.layer1 },
        "--dsw-specific-selector": { light: L.layer3, dark: D.layer3 },
        "--dsw-specific-input-major": { light: L.layer1, dark: D.layer1 },
        "--dsw-specific-tip": { light: L.layer3, dark: D.layer3 },
        "--dsw-alias-toast-bg": { light: "#2A2A2E", dark: "#2A2A2E" },
        "--dsw-alias-tooltip-bg": { light: "#2A2A2E", dark: "#2A2A2E" },
        // 滚动条（细窄中性）
        "--dsw-alias-scrollbar-bg-l1": { light: "#D0D0D4", dark: "#3A3A40" },
        "--dsw-alias-scrollbar-bg-l2": { light: "#D0D0D4", dark: "#3A3A40" },
        "--dsw-alias-scrollbar-hover-l1": { light: "#B8B8BE", dark: "#4A4A50" },
        "--dsw-alias-scrollbar-hover-l2": { light: "#B8B8BE", dark: "#4A4A50" },
      };
    }

    // ── 设置命名空间 ──
    const SETTINGS_NS = "dsh-rtui";
    const THEME_SOURCE = "dsh-rtui-ui";

    function createCustomStore() {
      return defineStore({
        init: () => ({
          enabled: true, preset: "graphite", accent: "#4D6BFE", sidebarContrast: "slightly",
          revision: -1,
        }),
        actions: {
          sync: (d, values, revision) => {
            if (revision <= d.revision) return;
            d.enabled = values.enabled !== false;
            d.preset = values.preset || "graphite";
            d.accent = values.accent || "#4D6BFE";
            d.sidebarContrast = values.sidebarContrast || "slightly";
            d.revision = revision;
          },
        },
      });
    }

    const rowStyle = { display: "flex", alignItems: "center", justifyContent: "space-between", padding: "10px 0", borderBottom: "1px solid var(--dsw-alias-border-l2)", gap: 12 };
    const labelStyle = { color: "var(--dsw-alias-label-primary)", fontSize: 13 };
    const selectStyle = { background: "var(--dsw-alias-bg-layer-1)", color: "var(--dsw-alias-label-primary)", border: "1px solid var(--dsw-alias-border-l2)", borderRadius: 6, padding: "4px 8px", fontSize: 13 };

    function Swatch({ color, active, onClick, title }) {
      return h("button", {
        type: "button", title,
        onClick,
        style: {
          width: 22, height: 22, borderRadius: 6, cursor: "pointer", background: color,
          border: active ? "2px solid var(--dsw-alias-label-primary)" : "1px solid var(--dsw-alias-border-l2)",
        },
      });
    }

    function ControlRow({ label, children }) {
      return h("div", { style: rowStyle },
        h("span", { style: labelStyle }, label),
        children,
      );
    }

    function CustomThemeRow({ t, useStore, setEnabled, setPreset, setAccent, setSidebarContrast }) {
      const enabled = useStore((s) => s.enabled);
      const preset = useStore((s) => s.preset);
      const accent = useStore((s) => s.accent);
      const sidebarContrast = useStore((s) => s.sidebarContrast);
      return h("div", { style: { padding: "8px 0" } },
        h("div", { style: { color: "var(--dsw-alias-label-primary)", fontSize: 14, fontWeight: 600, marginBottom: 8 } }, t("dsh-rtui.title")),
        ControlRow({ label: t("dsh-rtui.enable"),
          children: h("input", { type: "checkbox", checked: !!enabled, onChange: (e) => setEnabled(e.target.checked) }) }),
        ControlRow({ label: t("dsh-rtui.preset"),
          children: h("select", { value: preset, onChange: (e) => setPreset(e.target.value), style: selectStyle },
            PRESETS.map((c) => h("option", { key: c.id, value: c.id }, c.label))) }),
        ControlRow({ label: t("dsh-rtui.accent"),
          children: h("div", { style: { display: "flex", alignItems: "center", gap: 8 } },
            ACCENTS.map((a) => h(Swatch, { key: a.id, color: a.id, active: accent.toLowerCase() === a.id.toLowerCase(), title: a.label, onClick: () => setAccent(a.id) })),
            h("input", { type: "color", value: accent, onChange: (e) => setAccent(e.target.value), style: { width: 28, height: 24, border: "none", background: "none", cursor: "pointer" } }),
          ) }),
        ControlRow({ label: t("dsh-rtui.sidebarContrast"),
          children: h("select", { value: sidebarContrast, onChange: (e) => setSidebarContrast(e.target.value), style: selectStyle },
            CONTRASTS.map((c) => h("option", { key: c.id, value: c.id }, c.label))) }),
      );
    }

    const zh = {
      "dsh-rtui.title": "自定义主题",
      "dsh-rtui.enable": "启用优化主题",
      "dsh-rtui.preset": "主题预设",
      "dsh-rtui.accent": "强调色",
      "dsh-rtui.sidebarContrast": "侧栏对比度",
    };
    const en = {
      "dsh-rtui.title": "Custom theme",
      "dsh-rtui.enable": "Enable optimized theme",
      "dsh-rtui.preset": "Preset",
      "dsh-rtui.accent": "Accent color",
      "dsh-rtui.sidebarContrast": "Sidebar contrast",
    };

    // ── 结构性 CSS：token 覆盖不了的部分 ──
    function buildCss({ accent }) {
      const ac = accent || "#4D6BFE";
      return `
:root {
  /* 顶部右侧为 frameless 窗口的 Windows 系统按钮预留空间,避让 dsh 头部工具区。 */
  --rtui-sysbar: 128px;
  --rtui-accent: ${ac};
}
/* 焦点环(无障碍): 用强调色描边键盘焦点 */
button:focus-visible, a:focus-visible, input:focus-visible, textarea:focus-visible, select:focus-visible, [role="button"]:focus-visible, [tabindex]:focus-visible { outline: 2px solid var(--rtui-accent); outline-offset: 2px; }
/* 主题切换过渡: 颜色/背景/边框柔和过渡,避免瞬间跳变 */
body, [class*="App"], [class*="Panel"], [class*="Card"], [class*="Row"], [class*="Item"], button, input, select, textarea { transition: background-color .18s ease, color .18s ease, border-color .18s ease; }
/* 收起侧边栏: 侧栏颜色与主题底色一致 + 去掉右边框(macOS 下与红绿灯更协调)。
   侧栏列根节点收起时带 collapsed 类(哈希后仍含字面量 "collapsed")；
   右边框在布局网格列 [class*="sidebarCol"] 上,用 :has() 依据内部收起状态抹掉。 */
[data-slot="sidebar"] > :first-child[class*="collapsed"],
[class*="sidebarCol"]:has([data-slot="sidebar"] > :first-child[class*="collapsed"]) {
  background: var(--dsw-alias-bg-base) !important;
  border-right: none !important;
}
/* 滚动条细窄化 */
::-webkit-scrollbar { width: 8px; height: 8px; }
::-webkit-scrollbar-thumb { border-radius: 4px; }
/* 头部右上工具区右移,避让 Windows 系统按钮 */
.wSkVaW_headerUtilities { padding-right: var(--rtui-sysbar); }
`;
    }

    let lastValues = null;
    function injectStyle() {
      if (!lastValues) return;
      const existing = document.querySelector('style[data-plugin-css="dsh-rtui-ui"]');
      if (existing) existing.remove();
      const style = document.createElement("style");
      style.setAttribute("data-plugin-css", "dsh-rtui-ui");
      style.textContent = buildCss(lastValues);
      document.head.appendChild(style);
    }

    function apply(ctx) {
      const scope = ctx.settingsScope.bind({ namespace: SETTINGS_NS });
      ctx.locale.register(SETTINGS_NS, { zh, en });
      const store = createCustomStore();
      let bound;
      let disposeLayer = null;
      const applyTheme = (values) => {
        if (disposeLayer) { disposeLayer(); disposeLayer = null; }
        if (values.enabled === false) return;
        const tokens = expandTokens(values.preset, {
          accent: values.accent || "#4D6BFE",
          sidebarContrast: values.sidebarContrast || "slightly",
        });
        disposeLayer = ctx.theme.overrideTokens(THEME_SOURCE, tokens);
      };
      const applyFromSnapshot = (snap) => {
        const user = (snap && snap.user) || {};
        const values = {
          enabled: user.enabled !== false,
          preset: user.preset || "graphite",
          accent: user.accent || "#4D6BFE",
          sidebarContrast: user.sidebarContrast || "slightly",
        };
        lastValues = values;
        applyTheme(values);
        injectStyle();
        bound?.sync(values, snap ? snap.revision : -1);
      };
      scope.subscribe(applyFromSnapshot);
      const injected = (actions) => {
        bound = actions;
        applyFromSnapshot(scope.getSnapshot());
        return {
          setEnabled: (v) => scope.set("enabled", v),
          setPreset: (v) => scope.set("preset", v),
          setAccent: (v) => scope.set("accent", v),
          setSidebarContrast: (v) => scope.set("sidebarContrast", v),
        };
      };
      ctx.slots.inject("settings.general.item", () => ctx.slots.register({
        name: "settings.general.item",
        id: "dsh-rtui-theme",
        order: 20,
        store,
        locale: SETTINGS_NS,
        inject: injected,
      }, CustomThemeRow));
      applyFromSnapshot(scope.getSnapshot());
    }

    exports.apply = apply;
    exports.inject = ["theme", "slots", "locale", "settingsScope", "connection", "remote"];
    return module.exports;
  },
});
