import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import tailwindcss from "@tailwindcss/vite";
import { resolve } from "path";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [vue(), tailwindcss()],
  resolve: {
    alias: {
      "@": resolve(__dirname, "src"),
    },
  },
  clearScreen: false,
  build: {
    rollupOptions: {
      output: {
        // TDZ 防御：把入口 chunk 与懒加载 view chunk 共享的运行时模块
        // （stores / locales / services / pinia / vue-i18n / tauri / lucide / vue-router）
        // 强制下沉到独立 chunk，使 view chunk 不再静态 import 入口 chunk，
        // 从根上消除「入口 --动态--> view --静态--> 入口」的跨 chunk 环
        // （该环在 rolldown 分块/内联顺序变化或旧版 WebView2 下可产生
        //   `Cannot access 'X' before initialization` TDZ）。
        manualChunks(id: string) {
          if (id.includes("node_modules/@lucide/vue")) return "vendor-icons";
          if (id.includes("node_modules/vue-router")) return "vendor-router";
          if (
            id.includes("node_modules/vue-i18n") ||
            id.includes("node_modules/pinia") ||
            id.includes("node_modules/@tauri-apps")
          ) {
            return "app-core";
          }
          if (
            id.includes("/src/stores/") ||
            id.includes("/src/locales/") ||
            id.includes("/src/services/") ||
            id.includes("/src/components/common/")
          ) {
            return "app-core";
          }
        },
      },
    },
  },
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));
