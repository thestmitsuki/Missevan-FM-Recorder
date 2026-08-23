<script setup lang="ts">
/**
 * 全局播放条（跨路由显示）：挂在 AppLayout（主窗口布局，RouterView 外层），
 * 任意路由页面均可看到并控制播放。音频生命周期全局（playerStore 单例
 * audio 挂 document.body）：页面切换不中断播放，播放条 UI 从 store 状态
 * 实时恢复（切回任意页面都持续可见）。
 */
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { Pause, Play, Volume2, X } from "@lucide/vue";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { Slider } from "@/components/ui/slider";
import { usePlayerStore } from "@/stores/playerStore";

const { t } = useI18n();
const player = usePlayerStore();

/** 秒 → mm:ss（与 FilesView 原实现一致） */
function formatTime(sec: number): string {
    if (!isFinite(sec) || sec < 0) return "0:00";
    const m = Math.floor(sec / 60);
    const s = Math.floor(sec % 60);
    return `${m}:${String(s).padStart(2, "0")}`;
}

/** 进度条拖动 → seek（Slider 返回 number[]，取首值比例） */
function seekAudio(value: number[]) {
    const next = value[0] ?? 0;
    if (player.duration) player.seek(next / player.duration);
}

/** 音量滑块双向绑定（Slider 组件，跟随主题；setVolume 同步到 audio） */
const playerVolume = computed({
    get: () => [player.volume],
    set: (v: number[]) => {
        player.setVolume(v[0] ?? 0);
    },
});
</script>

<template>
    <Card
        v-if="player.currentFile"
        class="fixed bottom-4 left-1/2 z-50 flex w-[min(560px,calc(100vw-2rem))] -translate-x-1/2 flex-row items-center gap-3 rounded-xl border-border/70 bg-background/95 p-3 shadow-lg backdrop-blur"
        :aria-label="t('player.nowPlaying')"
    >
        <Button
            size="icon"
            variant="ghost"
            class="size-9 shrink-0 rounded-full"
            :aria-label="player.playing ? t('player.pause') : t('player.play')"
            @click="player.togglePlay()"
        >
            <Pause v-if="player.playing" class="size-4" />
            <Play v-else class="size-4" />
        </Button>

        <div class="min-w-0 flex-1">
            <div class="flex items-center gap-2">
                <p class="truncate text-xs font-semibold">
                    {{ player.currentFile.name }}
                </p>
                <Badge
                    v-if="player.isQueuePlay"
                    class="shrink-0 border-transparent bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary"
                >
                    {{
                        t("player.playerProgress", {
                            current: player.queueIndex + 1,
                            total: player.queue.length,
                        })
                    }}
                </Badge>
            </div>
            <div class="mt-1 flex items-center gap-2">
                <span
                    class="w-9 shrink-0 text-right text-[10px] tabular-nums text-muted-foreground"
                >
                    {{ formatTime(player.currentTime) }}
                </span>
                <Slider
                    :model-value="[player.currentTime]"
                    :min="0"
                    :max="player.duration || 0"
                    :step="0.1"
                    class="h-1.5 flex-1"
                    :aria-label="t('player.playerSeek')"
                    @update:model-value="(v: number[] | undefined) => seekAudio(v ?? [])"
                />
                <span
                    class="w-9 shrink-0 text-right text-[10px] tabular-nums text-muted-foreground"
                >
                    {{ formatTime(player.duration) }}
                </span>
            </div>
        </div>

        <div class="flex shrink-0 items-center gap-1">
            <Volume2
                class="size-3.5 text-muted-foreground"
                aria-hidden="true"
            />
            <Slider
                v-model="playerVolume"
                :min="0"
                :max="1"
                :step="0.01"
                :aria-label="t('player.playerVolume')"
                class="w-14"
            />
            <Button
                size="icon"
                variant="ghost"
                class="size-8 rounded-full"
                :aria-label="t('common.close')"
                @click="player.stopPlayback()"
            >
                <X class="size-4" />
            </Button>
        </div>
    </Card>
</template>
