import pluginVue from "eslint-plugin-vue";
import tseslint from "typescript-eslint";
import vueParser from "vue-eslint-parser";

export default tseslint.config(
  {
    ignores: [
      "dist/**",
      "node_modules/**",
      "src-tauri/**",
      "shadcn-vue-dev/**",
      "docs/**",
      "ARCH/**",
      "备份/**",
      "GitHub/**",
      "packaging/**",
      "public/**",
      "ffmpeg/**",
      "*.tsbuildinfo",
      "**/*.d.ts",
      "**/*.d.ts.map",
    ],
  },
  // ── Vue SFC：vue-eslint-parser + TS 脚本解析（在 essential 规则前声明）──
  {
    files: ["**/*.vue"],
    languageOptions: {
      parser: vueParser,
      parserOptions: {
        parser: tseslint.parser,
        extraFileExtensions: [".vue"],
      },
    },
  },
  ...pluginVue.configs["flat/essential"],
  {
    // shadcn 风格 UI 组件为单字命名（Badge/Button/Sheet 等），属刻意命名约定
    files: [
      "src/components/ui/**/*.vue",
      "src/components/common/Toast.vue",
    ],
    rules: {
      "vue/multi-word-component-names": "off",
    },
  },
  {
    rules: {
      // 防 TDZ 回归：const/let 在声明前被引用即报错。
      // functions/classes 为提升声明，不检查；variables 覆盖 let/const/var。
      // 背景：FilesView 曾因 watch(flatItems) 同步求值早于 collapsed 声明
      // 触发 "Cannot access 'collapsed' before initialization"（二次进入必现）。
      "no-use-before-define": [
        "error",
        { functions: false, classes: false, variables: true },
      ],
      // settings 各 section 接收深响应式 config 对象 prop 并就地修改其属性
      // （对象引用共享，属刻意模式），关闭 prop 属性变更误报。
      "vue/no-mutating-props": "off",
    },
  },
);
