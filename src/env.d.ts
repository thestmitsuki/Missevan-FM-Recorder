/// <reference types="vite/client" />

declare module "*.vue" {
  import type { DefineComponent } from "vue";
  const component: DefineComponent<object, object, unknown>;
  export default component;
}

// vue-i18n type augmentation — allows defining custom message schema
import "vue-i18n";
declare module "vue-i18n" {
  export interface DefineLocaleMessage {
    [key: string]: any;
  }
}
