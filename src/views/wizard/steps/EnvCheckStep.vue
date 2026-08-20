<script setup lang="ts">
/**
 * 向导第三页：环境检查（规格「引导菜单」第三页）
 *
 * 进入页面时自动并发检查 4 项（FFmpeg / ffprobe / 磁盘空间 / 写入权限），
 * 每项以卡片展示；FFmpeg/ffprobe 异常提供「下载并安装」+ 进度条 + 手动下载链接；
 * 磁盘/写入异常提供「更改输出目录」（跳回第二页）；关键工具未过时「下一步」禁用
 * 并提示「请先解决错误项」。
 *
 * 写入时机（修复子代理 B）：本页**只做环境检查与 FFmpeg 下载，绝不落盘**——
 * 配置文件的唯一写入点在向导最后一步「完成」按钮（CompleteStep 先 saveConfig
 * 全量落盘、再 finishWizard）。FFmpeg 下载成功后路径暂存到 wizardStore.staged
 * （download_ffmpeg 后端已不再写配置），完成时随 stagedToConfigPatch 一并写入。
 * 中途任意步骤退出 → 无配置文件 → 下次启动仍进向导。
 */
import { computed, onMounted, onUnmounted, reactive, ref } from "vue";
import { useI18n } from "vue-i18n";
import { storeToRefs } from "pinia";
import {
  CheckCircle,
  CircleX,
  Download,
  LoaderCircle,
  RotateCw,
  TriangleAlert,
} from "@lucide/vue";

import { api } from "@/services/api";
import { onDownloadProgress } from "@/services/events";
import { useWizardStore } from "@/stores/wizardStore";
import type { CheckResult, CheckStatus } from "@/types/health";

import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { Skeleton } from "@/components/ui/skeleton";

const { t } = useI18n();
const wizardStore = useWizardStore();
const { staged } = storeToRefs(wizardStore);

const emit = defineEmits<{
  previous: [];
  next: [];
  changeOutputDir: [];
}>();

// ── 检查状态 ──
type CardKey = "ffmpeg" | "ffprobe" | "disk" | "write";

interface CheckCard {
  key: CardKey;
  labelKey: string;
  status: CheckStatus;
  message: string;
}

const CARD_SLOTS: { key: CardKey; labelKey: string }[] = [
  { key: "ffmpeg", labelKey: "wizard.checkFfmpeg" },
  { key: "ffprobe", labelKey: "wizard.checkFfprobe" },
  { key: "disk", labelKey: "wizard.checkDisk" },
  { key: "write", labelKey: "wizard.checkWrite" },
];

const checking = ref(false);
const checkError = ref<string | null>(null);
const cards = ref<CheckCard[]>([]);

// ── FFmpeg 下载状态 ──
const download = reactive<{
  active: boolean;
  percent: number;
  stage: string;
  error: string | null;
}>({
  active: false,
  percent: 0,
  stage: "",
  error: null,
});

let unlistenProgress: (() => void) | null = null;

// ── 卡片状态派生 ──
const ffmpegCard = computed(() => cards.value.find((c) => c.key === "ffmpeg"));
const ffprobeCard = computed(() => cards.value.find((c) => c.key === "ffprobe"));

/** 关键工具（FFmpeg/ffprobe）通过 → 才允许下一步 */
const toolsOk = computed(
  () =>
    ffmpegCard.value?.status === "Passed" &&
    ffprobeCard.value?.status === "Passed",
);

// L6 审查跟进：磁盘低于阈值时后端返回 Failed、前端降级为 Warning（黄色，
// 不阻塞下一步，提供「更改输出目录」入口）；下一步可用判定 = toolsOk。

// ── 检查执行 ──
function buildCards(results: CheckResult[]): CheckCard[] {
  const list: CheckCard[] = [];
  const seen = new Set<CardKey>();

  for (const r of results) {
    const name = r.check_name.toLowerCase();
    let key: CardKey;
    if (name.includes("ffmpeg")) key = "ffmpeg";
    else if (name.includes("ffprobe")) key = "ffprobe";
    // 后端检查项名「磁盘空间/磁盘可用空间」为中文，须以 \u 转义匹配（避免源码内硬编码中文）
    else if (name.includes("\u78c1\u76d8") || name.includes("disk")) key = "disk";
    else key = "write";
    if (seen.has(key)) continue;
    seen.add(key);
    let status: CheckStatus = r.status;
    // 磁盘低于阈值（后端返回 Failed）→ 显示为警告（黄），不阻塞下一步；
    // FFmpeg/ffprobe 的 Failed 保持异常（红）并由 toolsOk 阻塞下一步
    if (key === "disk" && status === "Failed") status = "Warning";
    const slot = CARD_SLOTS.find((s) => s.key === key)!;
    list.push({ key, labelKey: slot.labelKey, status, message: r.message });
  }

  // 后端未返回的槽位兜底（正常不会发生）
  for (const slot of CARD_SLOTS) {
    if (!seen.has(slot.key)) {
      list.push({ key: slot.key, labelKey: slot.labelKey, status: "Skipped", message: "—" });
    }
  }
  return list;
}

async function runChecks() {
  checking.value = true;
  checkError.value = null;
  try {
    const report = await api.runWizardHealthCheck(
      staged.value.outputDir.trim(),
      staged.value.diskThresholdGb,
    );
    cards.value = buildCards(report.results);
    // 写入时机（修复子代理 B）：本页只做环境检查，不再自动落盘——
    // 配置文件的唯一写入点在向导最后一步「完成」按钮（CompleteStep.saveConfig）
  } catch (e) {
    checkError.value = String(e);
  } finally {
    checking.value = false;
  }
}

// ── FFmpeg 下载 ──
async function startDownload() {
  if (download.active) return;
  download.active = true;
  download.error = null;
  download.percent = 0;
  download.stage = "";
  try {
    const result = await api.downloadFfmpeg();
    // 下载路径暂存到 wizardStore（后端不再写配置）：完成时随 stagedToConfigPatch 写入
    wizardStore.setStaged({
      ffmpegPath: result.ffmpeg_path,
      ffprobePath: result.ffprobe_path,
    });
    download.stage = "done";
    // 下载完成后自动再次检测（规格）
    await runChecks();
  } catch (e) {
    download.error = String(e);
  } finally {
    download.active = false;
  }
}

// ── 下一步（仅要求关键工具通过；配置在最后一步统一落盘） ──
function handleNext() {
  if (!toolsOk.value) return;
  emit("next");
}

onMounted(() => {
  runChecks();
  // 监听下载进度（统一 events.ts 层：onDownloadProgress，不直接 listen）
  unlistenProgress = onDownloadProgress((payload) => {
    download.percent = payload.percent;
    download.stage = payload.stage;
  });
});

onUnmounted(() => {
  unlistenProgress?.();
});

// ── 状态展示辅助 ──
function statusIcon(status: CheckStatus) {
  switch (status) {
    case "Passed":
      return CheckCircle;
    case "Warning":
      return TriangleAlert;
    case "Failed":
      return CircleX;
    default:
      return LoaderCircle;
  }
}

function statusClass(status: CheckStatus): string {
  switch (status) {
    case "Passed":
      return "text-emerald-600 dark:text-emerald-500";
    case "Warning":
      return "text-amber-600 dark:text-amber-500";
    case "Failed":
      return "text-destructive";
    default:
      return "text-muted-foreground";
  }
}

function statusLabel(status: CheckStatus): string {
  switch (status) {
    case "Passed":
      return t("wizard.statusOk");
    case "Warning":
      return t("wizard.statusWarning");
    case "Failed":
      return t("wizard.statusError");
    default:
      return t("wizard.statusChecking");
  }
}

function stageLabel(stage: string): string {
  if (!stage) return "";
  const key = `wizard.downloadStage.${stage}`;
  const translated = t(key);
  return translated === key ? "" : translated;
}
</script>

<template>
  <div class="flex flex-1 flex-col">
    <h2 class="text-xl font-semibold tracking-tight">
      {{ t("wizard.envTitle") }}
    </h2>
    <p class="mt-1 text-sm text-muted-foreground">
      {{ t("wizard.envDesc") }}
    </p>

    <!-- 检查失败（命令级错误） -->
    <div
      v-if="checkError"
      class="mt-5 flex items-center justify-between rounded-lg border border-destructive/40 bg-destructive/10 px-4 py-3"
      role="alert"
    >
      <span class="text-sm text-destructive">{{ t("wizard.checkFailed", { error: checkError }) }}</span>
      <Button variant="outline" size="sm" @click="runChecks">
        <RotateCw class="size-3.5" />
        {{ t("wizard.retryCheck") }}
      </Button>
    </div>

    <!-- 检查列表（加载骨架 / 卡片） -->
    <div class="mt-6 flex-1 space-y-4">
      <template v-if="checking && cards.length === 0">
        <div
          v-for="i in 4"
          :key="i"
          class="rounded-lg border p-4"
        >
          <Skeleton class="h-5 w-40" />
          <Skeleton class="mt-3 h-4 w-full" />
          <Skeleton class="mt-2 h-4 w-2/3" />
        </div>
      </template>

      <template v-else>
        <div
          v-for="card in cards"
          :key="card.key"
          class="rounded-lg border bg-card p-4"
        >
          <div class="flex items-center justify-between">
            <span class="text-sm font-medium">{{ t(card.labelKey) }}</span>
            <span class="flex items-center gap-1.5 text-sm" :class="statusClass(card.status)">
              <component :is="statusIcon(card.status)" :class="{ 'animate-spin': card.status !== 'Passed' && card.status !== 'Failed' && card.status !== 'Warning' }" class="size-4" />
              {{ statusLabel(card.status) }}
            </span>
          </div>
          <p class="mt-2 break-all text-xs leading-relaxed text-muted-foreground">
            {{ card.message }}
          </p>

          <!-- FFmpeg / ffprobe 异常 → 下载并安装（同时修复两个工具） -->
          <div
            v-if="(card.key === 'ffmpeg' || card.key === 'ffprobe') && card.status === 'Failed'"
            class="mt-3"
          >
            <Button
              variant="outline"
              size="sm"
              :disabled="download.active"
              @click="startDownload"
            >
              <LoaderCircle v-if="download.active" class="size-3.5 animate-spin" />
              <Download v-else class="size-3.5" />
              {{ t("wizard.downloadInstall") }}
            </Button>
          </div>

          <!-- 磁盘 / 写入异常 → 更改输出目录 -->
          <div
            v-if="(card.key === 'disk' || card.key === 'write') && (card.status === 'Failed' || card.status === 'Warning')"
            class="mt-3"
          >
            <Button variant="outline" size="sm" @click="emit('changeOutputDir')">
              {{ t("wizard.changeOutputDir") }}
            </Button>
          </div>
        </div>
      </template>

      <!-- 下载进度条 -->
      <div
        v-if="download.active"
        class="rounded-lg border bg-card p-4"
        role="progressbar"
        :aria-valuenow="download.percent"
        aria-valuemin="0"
        aria-valuemax="100"
      >
        <div class="mb-2 flex items-center justify-between text-xs text-muted-foreground">
          <span>{{ stageLabel(download.stage) || t("wizard.downloading") }}</span>
          <span>{{ download.percent }}%</span>
        </div>
        <Progress :model-value="download.percent" />
      </div>

      <!-- 下载失败：错误 + 手动下载链接 -->
      <div
        v-if="download.error"
        class="rounded-lg border border-destructive/40 bg-destructive/10 px-4 py-3"
        role="alert"
      >
        <p class="text-xs text-destructive">
          {{ t("wizard.downloadFailed", { error: download.error }) }}
        </p>
        <p class="mt-1.5 text-xs text-muted-foreground">
          {{ t("wizard.manualDownloadHint") }}
          <a
            href="https://ffmpeg.org/download.html"
            target="_blank"
            rel="noopener noreferrer"
            class="font-medium text-primary underline underline-offset-2"
          >
            {{ t("wizard.manualDownload") }}
          </a>
        </p>
        <Button variant="outline" size="sm" class="mt-2" :disabled="download.active" @click="startDownload">
          <RotateCw class="size-3.5" />
          {{ t("wizard.retryDownload") }}
        </Button>
      </div>
    </div>

    <!-- 底部按钮 -->
    <div class="mt-8 flex items-center justify-between">
      <Button variant="outline" @click="emit('previous')">
        {{ t("wizard.previous") }}
      </Button>
      <div class="flex items-center gap-3">
        <span
          v-if="!toolsOk && cards.length > 0"
          class="text-xs font-medium text-destructive"
          role="alert"
        >
          {{ t("wizard.fixErrorsFirst") }}
        </span>
        <Button class="min-w-28" :disabled="!toolsOk" @click="handleNext">
          {{ t("wizard.next") }}
        </Button>
      </div>
    </div>
  </div>
</template>
