import { createRouter, createWebHashHistory } from "vue-router";
import { isWizardWindow } from "@/services/window";
import { useDebugStore } from "@/stores/debugStore";

const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    {
      path: "/",
      name: "live",
      component: () => import("../views/live/LiveView.vue"),
      meta: { title: "nav.liveMonitor" },
    },
    {
      path: "/files",
      name: "files",
      component: () => import("../views/files/FilesView.vue"),
      meta: { title: "nav.fileManager" },
    },
    {
      path: "/settings",
      name: "settings",
      component: () => import("../views/settings/SettingsView.vue"),
      meta: { title: "nav.settings" },
    },
    {
      path: "/wizard",
      name: "wizard",
      component: () => import("../views/wizard/WizardView.vue"),
      meta: { title: "nav.setupWizard" },
    },
    {
      path: "/debug",
      name: "debug",
      component: () => import("../views/debug/DebugView.vue"),
      meta: { title: "nav.debugPanel" },
    },
  ],
});

// ── 双窗口路由守卫（M2）──
// 向导窗口（label=wizard）强制进入 /wizard，避免 setup 期先渲染主页面导致的闪烁；
// 主窗口 / 浏览器调试环境保持原有路由行为
// ── 调试页守卫（B3）──
// 调试模式默认关闭（设置 → 高级 开关，localStorage `debug_enabled`）：
// 未开启时导航「更多」中不显示调试入口，直接访问 /debug 也重定向回直播监控页
router.beforeEach((to) => {
  if (isWizardWindow() && to.name !== "wizard") {
    return { name: "wizard" };
  }
  if (to.name === "debug" && !useDebugStore().enabled) {
    return { name: "live" };
  }
  return true;
});

export default router;
