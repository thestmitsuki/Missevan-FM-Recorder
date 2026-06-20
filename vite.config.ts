import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

export default defineConfig({
  plugins: [vue()],
  server: {
    watch: {
      ignored: ["**/src-tauri/target/**"], // 忽略 Rust 构建目录
    },
  },
});
