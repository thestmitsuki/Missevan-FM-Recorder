/**
 * 设置页 8 分类注册表（规格「设置页面功能规格」）
 *
 * 每分类：id / i18n key / 图标 / 组件 / 归属的 GlobalConfig 字段（用于「恢复默认值」）。
 * 外观（localStorage）与快捷键（localStorage）无 GlobalConfig 字段，恢复逻辑单独处理。
 */
import type { Component } from "vue";
import {
    Bell,
    Cpu,
    FolderCog,
    Globe,
    Keyboard,
    Palette,
    Radio,
    Settings2,
    type LucideIcon,
} from "@lucide/vue";
import type { GlobalConfig } from "@/types";
import GeneralSection from "./GeneralSection.vue";
import RecordingSection from "./RecordingSection.vue";
import FileSection from "./FileSection.vue";
import NetworkSection from "./NetworkSection.vue";
import NotificationSection from "./NotificationSection.vue";
import AppearanceSection from "./AppearanceSection.vue";
import ShortcutSection from "./ShortcutSection.vue";
import AdvancedSection from "./AdvancedSection.vue";

export type CategoryId =
    | "general"
    | "recording"
    | "files"
    | "network"
    | "notification"
    | "appearance"
    | "shortcuts"
    | "advanced";

export interface SettingsCategory {
    id: CategoryId;
    /** settings.categories.<id> */
    labelKey: string;
    /** settings.categoryDesc.<id> */
    descKey: string;
    icon: Component | LucideIcon;
    component: Component;
    /** 归属该分类的 GlobalConfig 字段（恢复默认值时逐字段写回 DEFAULT_CONFIG） */
    fields: (keyof GlobalConfig)[];
}

export const CATEGORIES: SettingsCategory[] = [
    {
        id: "general",
        labelKey: "settings.categories.general",
        descKey: "settings.categoryDesc.general",
        icon: Settings2,
        component: GeneralSection,
        fields: ["autostart", "close_behavior", "show_tray", "check_updates"],
    },
    {
        id: "recording",
        labelKey: "settings.categories.recording",
        descKey: "settings.categoryDesc.recording",
        icon: Radio,
        component: RecordingSection,
        fields: [
            "output_dir",
            "record_format",
            "bitrate_kbps",
            "audio_only",
            "segment_seconds",
            "filename_template",
            "max_concurrent_recordings",
            "pre_record_delay_secs",
            "post_record_action",
            "post_record_command",
        ],
    },
    {
        id: "files",
        labelKey: "settings.categories.files",
        descKey: "settings.categoryDesc.files",
        icon: FolderCog,
        component: FileSection,
        fields: [
            "auto_cleanup_enabled",
            "retention_days",
            "max_total_gb",
            // cleanup_time 每日定时已废弃（自动清理改为录制结束时触发），不再展示
            "disk_space_limit_gb",
        ],
    },
    {
        id: "network",
        labelKey: "settings.categories.network",
        descKey: "settings.categoryDesc.network",
        icon: Globe,
        component: NetworkSection,
        fields: [
            "proxy_type",
            "proxy_addr",
            "proxy_port",
            "proxy_auth",
            "proxy_username",
            "proxy_password",
            "api_timeout_secs",
            "stream_timeout_secs",
            "max_retries",
            "retry_delay_secs",
            "custom_dns",
        ],
    },
    {
        id: "notification",
        labelKey: "settings.categories.notification",
        descKey: "settings.categoryDesc.notification",
        icon: Bell,
        component: NotificationSection,
        fields: [
            "notifications_enabled",
            "notify_recording_start",
            "notify_recording_end",
            "notify_recording_error",
            "notify_live_start",
            "notify_live_end",
            "notify_disk_warning",
            "notify_update",
            "notify_system",
            "notify_sound",
        ],
    },
    {
        id: "appearance",
        labelKey: "settings.categories.appearance",
        descKey: "settings.categoryDesc.appearance",
        icon: Palette,
        component: AppearanceSection,
        fields: [],
    },
    {
        id: "shortcuts",
        labelKey: "settings.categories.shortcuts",
        descKey: "settings.categoryDesc.shortcuts",
        icon: Keyboard,
        component: ShortcutSection,
        fields: [],
    },
    {
        id: "advanced",
        labelKey: "settings.categories.advanced",
        descKey: "settings.categoryDesc.advanced",
        icon: Cpu,
        component: AdvancedSection,
        fields: [
            "log_level",
            "detector_concurrency",
            "check_interval_secs",
            "detector_jitter_secs",
            "ffmpeg_path",
            "ffprobe_path",
        ],
    },
];
