import { createApp } from "vue";
import App from "./App.vue";
import router from "./router";
import { createPinia } from "pinia";
import { i18n } from "@/locales";
import { setupEventListeners } from "@/services/events";
import { useConfigStore } from "@/stores/configStore";
import { useAppearanceStore } from "@/stores/appearanceStore";
import "./styles/index.css";
import "./styles/shadcn.css";
// sonner 通知样式
import "vue-sonner/style.css";

const app = createApp(App);
const pinia = createPinia();
app.use(pinia);
app.use(router);
app.use(i18n);
// 应用外观偏好（强调色/字体/密度，localStorage 即时生效；避免首屏闪烁）
useAppearanceStore();
app.mount("#app");

// ── 初始化 ──
setupEventListeners().then(() => {
  // 加载配置
  const configStore = useConfigStore();
  configStore.fetchConfig();
});
