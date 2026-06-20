import { reactive, readonly } from "vue";

export type NotificationLevel = "info" | "warning" | "error";

interface NotificationState {
  visible: boolean;
  message: string;
  level: NotificationLevel;
}

const state = reactive<NotificationState>({
  visible: false,
  message: "",
  level: "info",
});

let timer: number | null = null;

export function useNotification() {
  const show = (
    message: string,
    level: NotificationLevel = "info",
    duration: number = 3000,
  ) => {
    if (timer) clearTimeout(timer);
    state.message = message;
    state.level = level;
    state.visible = true;
    timer = setTimeout(() => {
      state.visible = false;
    }, duration);
  };

  const hide = () => {
    if (timer) clearTimeout(timer);
    state.visible = false;
  };

  return {
    notificationState: readonly(state),
    show,
    hide,
  };
}
