import { createApp } from "vue";
import { createPinia } from "pinia";
import { createI18n } from "vue-i18n";
import "./styles/index.css";
import App from "./App.vue";

// 导入语言包
import zhCN from "./locales/zh-CN";
import en from "./locales/en";

// 创建 i18n 实例
const i18n = createI18n({
  legacy: false, // 使用 Composition API 模式
  locale: "zh-CN", // 默认语言
  fallbackLocale: "zh-CN",
  messages: {
    "zh-CN": zhCN,
    en: en,
  },
});

// 创建 Pinia
const pinia = createPinia();

const app = createApp(App);

// 全局错误处理
app.config.errorHandler = (err, _instance, info) => {
  console.error("Vue 全局错误:", err, info);
  // 可以在这里调用通知 store 显示错误
};

app.use(pinia);
app.use(i18n);
app.mount("#app");
