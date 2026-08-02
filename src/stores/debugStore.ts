/**
 * 调试模式开关（B3：默认关闭；开启后主导航「更多」中出现调试面板入口）
 *
 * 规格：「调试页面……仅用于开发版本或用户主动开启的调试模式」——
 * 持久化键 `debug_enabled`（localStorage，纯前端偏好，不进 GlobalConfig），
 * 默认 false；设置页「高级」分类的开关负责开启/关闭，即时生效。
 */
import { defineStore } from "pinia";
import { ref } from "vue";

const DEBUG_ENABLED_KEY = "debug_enabled";

function readEnabled(): boolean {
    try {
        return localStorage.getItem(DEBUG_ENABLED_KEY) === "1";
    } catch {
        return false;
    }
}

export const useDebugStore = defineStore("debug", () => {
    const enabled = ref(readEnabled());

    function setEnabled(value: boolean) {
        enabled.value = value;
        try {
            if (value) localStorage.setItem(DEBUG_ENABLED_KEY, "1");
            else localStorage.removeItem(DEBUG_ENABLED_KEY);
        } catch {
            // 忽略持久化失败（仅在内存中生效）
        }
    }

    return { enabled, setEnabled };
});
