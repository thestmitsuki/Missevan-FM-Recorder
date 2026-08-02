/**
 * 外观偏好（D5：语言/主题/密度/字体/卡片显示项 → 前端 localStorage，不进 GlobalConfig）
 *
 * 持久化键 `appearance`（JSON），子项：
 * - accent：强调色 hex，改写 shadcn.css 的 `--primary`/`--ring`/`--primary-foreground`
 *   （inline style 覆盖 :root/.dark 变量块，亮暗均生效）
 * - fontSize：small=14px / medium=15px / large=16px（根字号 → rem 系 Tailwind 尺寸整体缩放）
 * - density：compact / standard / comfortable（html[data-density] → `--density-mult` CSS 变量）
 * - cardShowAvatar / cardShowTags / cardShowRoomId / cardShowStatusIcon：
 *   直播页主播卡片显示项（AnchorCard 消费）
 */
import { defineStore } from "pinia";
import { ref } from "vue";
import { useThemeStore } from "@/stores/themeStore";

export type Density = "compact" | "standard" | "comfortable";
export type FontSize = "small" | "medium" | "large";
export type CardOptionKey =
    | "cardShowAvatar"
    | "cardShowTags"
    | "cardShowRoomId"
    | "cardShowStatusIcon";

export interface AppearancePrefs {
    accent: string; // hex，如 "#2563eb"
    density: Density;
    fontSize: FontSize;
    cardShowAvatar: boolean;
    cardShowTags: boolean;
    cardShowRoomId: boolean;
    cardShowStatusIcon: boolean;
}

/** 品牌蓝（shadcn.css `:root --primary: oklch(0.55 0.2 250)` 的 hex 近似） */
export const DEFAULT_ACCENT = "#2563eb";

export const DEFAULT_APPEARANCE: AppearancePrefs = {
    accent: DEFAULT_ACCENT,
    density: "standard",
    fontSize: "medium",
    cardShowAvatar: true,
    cardShowTags: true,
    cardShowRoomId: true,
    cardShowStatusIcon: true,
};

const APPEARANCE_KEY = "appearance";

const FONT_SIZE_PX: Record<FontSize, string> = {
    small: "14px",
    medium: "15px",
    large: "16px",
};

function isLightColor(hex: string): boolean {
    const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
    if (!m) return false;
    const n = parseInt(m[1], 16);
    const r = (n >> 16) & 0xff;
    const g = (n >> 8) & 0xff;
    const b = n & 0xff;
    // 感知亮度（sRGB 线性化近似）
    const lum = 0.299 * r + 0.587 * g + 0.114 * b;
    return lum > 0.6;
}

function readPrefs(): AppearancePrefs {
    const base = { ...DEFAULT_APPEARANCE };
    try {
        const raw = localStorage.getItem(APPEARANCE_KEY);
        if (raw) {
            const parsed = JSON.parse(raw) as Partial<AppearancePrefs>;
            if (typeof parsed.accent === "string" && /^#[0-9a-f]{6}$/i.test(parsed.accent)) {
                base.accent = parsed.accent;
            }
            if (parsed.density === "compact" || parsed.density === "standard" || parsed.density === "comfortable") {
                base.density = parsed.density;
            }
            if (parsed.fontSize === "small" || parsed.fontSize === "medium" || parsed.fontSize === "large") {
                base.fontSize = parsed.fontSize;
            }
            if (typeof parsed.cardShowAvatar === "boolean") base.cardShowAvatar = parsed.cardShowAvatar;
            if (typeof parsed.cardShowTags === "boolean") base.cardShowTags = parsed.cardShowTags;
            if (typeof parsed.cardShowRoomId === "boolean") base.cardShowRoomId = parsed.cardShowRoomId;
            if (typeof parsed.cardShowStatusIcon === "boolean") base.cardShowStatusIcon = parsed.cardShowStatusIcon;
        }
    } catch {
        // localStorage 不可用时使用默认值
    }
    return base;
}

function applyPrefs(prefs: AppearancePrefs) {
    const root = document.documentElement;
    // 强调色 → 改写 shadcn 主题变量（inline style 优先于 :root/.dark 样式块）
    root.style.setProperty("--primary", prefs.accent);
    root.style.setProperty("--ring", prefs.accent);
    root.style.setProperty(
        "--primary-foreground",
        isLightColor(prefs.accent) ? "#0b0b0b" : "#ffffff",
    );
    // 字体大小 → 根字号（rem 系尺寸整体缩放，即时生效）
    root.style.fontSize = FONT_SIZE_PX[prefs.fontSize];
    // 列表密度 → data-density 属性（--density-mult 由全局 CSS 消费）
    root.dataset.density = prefs.density;
}

export const useAppearanceStore = defineStore("appearance", () => {
    const prefs = ref<AppearancePrefs>(readPrefs());

    function persist() {
        try {
            localStorage.setItem(APPEARANCE_KEY, JSON.stringify(prefs.value));
        } catch {
            // 忽略持久化失败
        }
    }

    function update(patch: Partial<AppearancePrefs>) {
        prefs.value = { ...prefs.value, ...patch };
        applyPrefs(prefs.value);
        persist();
    }

    /** 主播卡片显示项（直播页 AnchorCard 消费） */
    function setCardItem(key: CardOptionKey, value: boolean) {
        update({ [key]: value } as Partial<AppearancePrefs>);
    }

    /** 恢复全部外观默认（含主题 → 跟随系统） */
    function resetAll() {
        prefs.value = { ...DEFAULT_APPEARANCE };
        applyPrefs(prefs.value);
        persist();
        useThemeStore().setMode("system");
    }

    // 初始化时应用一次（覆盖 shadcn.css 默认主题变量）
    applyPrefs(prefs.value);

    return {
        prefs,
        update,
        setCardItem,
        resetAll,
    };
});
