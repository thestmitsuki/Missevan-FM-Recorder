/**
 * 设置页输入校验（规格「交互细节」：路径/数字范围/必填实时校验，红边框+提示）。
 *
 * 校验以字段名（= GlobalConfig snake_case key）为 key 输出错误消息；
 * 每分类一个 validator，SettingsView 深监听表单时全量重算（字段少、开销可忽略）。
 */
import type { GlobalConfig } from "@/types";
import type { AppLocale } from "@/locales";
import type { AppearancePrefs } from "@/stores/appearanceStore";
import type { ThemeMode } from "@/stores/themeStore";
import type { CategoryId } from "./sections";

/** 表单基础形态：ffmpeg_path 归一为 string（"" = 自动探测），保存时再转回 null */
export type SettingsFormBase = Omit<GlobalConfig, "ffmpeg_path"> & { ffmpeg_path: string };

/**
 * 设置页表单全量形态 = GlobalConfig 字段 + 三个「保存后生效」暂存字段：
 * - locale / theme / appearance 为纯前端偏好（localStorage），暂存于表单，
 *   save() 成功时统一提交（setLocale / themeStore.setMode / appearanceStore.update）；
 * - shortcuts 随表单整包落盘（save_config 写入 GlobalConfig.shortcuts）。
 */
export type SettingsForm = SettingsFormBase & {
    locale: AppLocale;
    theme: ThemeMode;
    appearance: AppearancePrefs;
};

/** 字段名 → 错误消息（i18n 文案） */
export type SectionErrors = Record<string, string>;

export type I18nT = (key: string, params?: Record<string, unknown>) => string;

/**
 * 保存时归一化：空 ffmpeg_path → null（后端 None = 自动探测）。
 * - ffmpeg_path null 防护（Critical-1）：恢复默认/导入链路曾可能把 null 注入表单，
 *   trim 前兜底，保证保存链路不抛 TypeError；
 * - anchor_ids 剔除（Important-2）：该字段由 Live 页维护，设置页保存不透传表单快照，
 *   updateConfig 合并时保留 configStore 当前值（后端现值）；
 * - locale/theme/appearance 剔除：纯前端偏好，由 save() 直接提交（I6），不进后端。
 */
export function normalizeConfig(
    form: SettingsForm,
): Omit<GlobalConfig, "anchor_ids"> {
    const { ffmpeg_path, locale, theme, appearance, ...rest } = form;
    void locale;
    void theme;
    void appearance;
    delete (rest as { anchor_ids?: string[] }).anchor_ids;
    return { ...rest, ffmpeg_path: (ffmpeg_path ?? "").trim() ? ffmpeg_path : null };
}

/** 从后端配置深拷贝出表单基础形态（ffmpeg_path null → ""；暂存字段由 SettingsView 注入） */
export function cloneForm(config: GlobalConfig): SettingsFormBase {
    const { ffmpeg_path, ...rest } = JSON.parse(JSON.stringify(config)) as GlobalConfig;
    return { ...rest, ffmpeg_path: ffmpeg_path ?? "" };
}

/** 语义化默认值（Task 3 对齐提示 ③）：0/"" 是「不限制/自动」，不得渲染成真实限制 */
export const SEMANTIC_ZERO_FIELDS: Partial<Record<keyof GlobalConfig, string>> = {
    max_total_gb: "settings.errors.zeroUnlimited", // 0 = 不限
    proxy_port: "settings.errors.proxyPortNotSet", // 0 = 未设置
    segment_seconds: "settings.errors.segmentZeroNoSplit", // 0 = 不分割
    ffprobe_path: "settings.errors.emptyAutoDetect",
    custom_dns: "settings.errors.emptySystemDns",
};

const isInt = (n: number) => Number.isSafeInteger(n);
const inRange = (n: number, min: number, max: number) => isInt(n) && n >= min && n <= max;

export const validators: Record<CategoryId, (form: SettingsForm, t: I18nT) => SectionErrors> = {
    general: () => ({}),

    recording: (form, t) => {
        const e: SectionErrors = {};
        if (!form.output_dir.trim()) {
            e.output_dir = t("settings.errors.outputDirRequired");
        }
        if (!inRange(form.segment_seconds, 0, 86400)) {
            e.segment_seconds = t("settings.errors.segmentRange");
        }
        if (!inRange(form.pre_record_delay_secs, 0, 86400)) {
            e.pre_record_delay_secs = t("settings.errors.delayRange");
        }
        if (!inRange(form.max_concurrent_recordings, 1, 32)) {
            e.max_concurrent_recordings = t("settings.errors.concurrencyRange");
        }
        if (![64, 128, 192, 256, 320].includes(form.bitrate_kbps)) {
            e.bitrate_kbps = t("settings.errors.bitrateInvalid");
        }
        if (!form.filename_template.trim()) {
            e.filename_template = t("settings.errors.templateRequired");
        }
        if (form.post_record_action === "command" && !form.post_record_command.trim()) {
            e.post_record_command = t("settings.errors.commandRequired");
        }
        return e;
    },

    files: (form, t) => {
        const e: SectionErrors = {};
        if (!inRange(form.retention_days, 1, 3650)) {
            e.retention_days = t("settings.errors.retentionRange");
        }
        // max_total_gb = 0 表示不限制（语义化默认值，合法）
        if (!inRange(form.max_total_gb, 0, 100000)) {
            e.max_total_gb = t("settings.errors.maxTotalRange");
        }
        // disk_space_limit_gb 为磁盘保护阈值，后端 is_valid 要求 > 0
        if (!inRange(form.disk_space_limit_gb, 1, 100000)) {
            e.disk_space_limit_gb = t("settings.errors.diskSpaceRange");
        }
        const m = /^(\d{2}):(\d{2})$/.exec(form.cleanup_time.trim());
        if (!m || Number(m[1]) > 23 || Number(m[2]) > 59) {
            e.cleanup_time = t("settings.errors.cleanupTimeInvalid");
        }
        return e;
    },

    network: (form, t) => {
        const e: SectionErrors = {};
        if (!inRange(form.api_timeout_secs, 1, 600)) {
            e.api_timeout_secs = t("settings.errors.timeoutRange");
        }
        if (!inRange(form.stream_timeout_secs, 1, 3600)) {
            e.stream_timeout_secs = t("settings.errors.streamTimeoutRange");
        }
        if (!inRange(form.max_retries, 0, 20)) {
            e.max_retries = t("settings.errors.retriesRange");
        }
        if (!inRange(form.retry_delay_secs, 0, 3600)) {
            e.retry_delay_secs = t("settings.errors.retryDelayRange");
        }
        if (form.proxy_type !== "none") {
            if (!form.proxy_addr.trim()) {
                e.proxy_addr = t("settings.errors.addrRequired");
            }
            // proxy_port=0 仅在 proxy_type=none 时是「未设置」；启用代理时必须是有效端口
            if (!inRange(form.proxy_port, 1, 65535)) {
                e.proxy_port = t("settings.errors.portRange");
            }
            if (form.proxy_auth && !form.proxy_username.trim()) {
                e.proxy_username = t("settings.errors.usernameRequired");
            }
        }
        if (form.custom_dns.trim() && /\s/.test(form.custom_dns)) {
            e.custom_dns = t("settings.errors.dnsInvalid");
        }
        return e;
    },

    notification: () => ({}),

    appearance: () => ({}),

    shortcuts: () => ({}),

    advanced: (form, t) => {
        const e: SectionErrors = {};
        if (!inRange(form.detector_concurrency, 1, 64)) {
            e.detector_concurrency = t("settings.errors.detectorRange");
        }
        // 后端 is_valid 要求 check_interval_secs >= 5
        if (!inRange(form.check_interval_secs, 5, 86400)) {
            e.check_interval_secs = t("settings.errors.intervalRange");
        }
        // 随机抖动上限（0 = 不抖动，合法）
        if (!inRange(form.detector_jitter_secs, 0, 86400)) {
            e.detector_jitter_secs = t("settings.errors.jitterRange");
        }
        // null 防护（Critical-1）：恢复默认/导入链路曾可能把 ffmpeg_path 注入为 null
        const ffmpegPath = form.ffmpeg_path ?? "";
        if (ffmpegPath.trim()) {
            // Windows 可执行文件：允许 exe/bat/cmd/ps1
            if (!/\.(exe|bat|cmd|ps1)$/i.test(ffmpegPath.trim())) {
                e.ffmpeg_path = t("settings.errors.ffmpegPathInvalid");
            }
        }
        if (form.ffprobe_path.trim()) {
            if (!/\.(exe|bat|cmd|ps1)$/i.test(form.ffprobe_path.trim())) {
                e.ffprobe_path = t("settings.errors.ffprobePathInvalid");
            }
        }
        return e;
    },
};

export function validateCategory(
    categoryId: CategoryId,
    form: SettingsForm,
    t: I18nT,
): SectionErrors {
    return validators[categoryId](form, t);
}

/** 全部分类错误（保存前全量校验） */
export function validateAll(form: SettingsForm, t: I18nT): Record<CategoryId, SectionErrors> {
    const all = {} as Record<CategoryId, SectionErrors>;
    for (const id of Object.keys(validators) as CategoryId[]) {
        all[id] = validators[id](form, t);
    }
    return all;
}
