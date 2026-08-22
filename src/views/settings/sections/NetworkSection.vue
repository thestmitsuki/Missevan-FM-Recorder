<script setup lang="ts">
/**
 * 网络与代理分类（规格 7.4）：
 * 全局代理（类型 无/HTTP/SOCKS5 + 地址/端口 + 认证开关 + 账号/密码）、
 * 连接设置（API 超时/流超时/重试次数/重试延迟）、自定义 DNS。
 *
 * 对齐提示 ③：proxy_port=0 仅在 proxy_type=none 时为「未设置」，UI 不渲染成真实限制
 * （启用代理时校验端口 1-65535）；custom_dns="" 表示系统 DNS。
 * 对齐提示 ⑤：proxy_password 默认掩码显示（type=password + 眼睛切换）。
 */
import { ref, toRef } from "vue";
import { useI18n } from "vue-i18n";
import { Eye, EyeOff } from "@lucide/vue";
import type { SectionErrors, SettingsForm } from "../validation";
import { useNumberField } from "../useNumberField";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Switch } from "@/components/ui/switch";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import NotEffectiveBadge from "@/components/common/NotEffectiveBadge.vue";

const props = defineProps<{
    config: SettingsForm;
    errors: SectionErrors;
}>();

const { t } = useI18n();

const { text: apiTimeoutText, invalid: apiTimeoutInvalid } = useNumberField(
    toRef(props.config, "api_timeout_secs"),
);
const { text: streamTimeoutText, invalid: streamTimeoutInvalid } = useNumberField(
    toRef(props.config, "stream_timeout_secs"),
);
const { text: retriesText, invalid: retriesInvalid } = useNumberField(
    toRef(props.config, "max_retries"),
);
const { text: retryDelayText, invalid: retryDelayInvalid } = useNumberField(
    toRef(props.config, "retry_delay_secs"),
);
const { text: proxyPortText, invalid: proxyPortInvalid } = useNumberField(
    toRef(props.config, "proxy_port"),
);

const showPassword = ref(false);

const proxyTypes = [
    { value: "none", labelKey: "settings.network.proxyNone" },
    { value: "http", labelKey: "settings.network.proxyHttp" },
    { value: "socks5", labelKey: "settings.network.proxySocks5" },
];

const proxyEnabled = () => props.config.proxy_type !== "none";
</script>

<template>
    <div class="space-y-6">
        <!-- ── 全局代理设置 ── -->
        <Card class="gap-0 rounded-lg p-4 shadow-none">
            <CardHeader class="mb-4 gap-0 p-0">
                <CardTitle class="text-sm font-semibold">
                    {{ t("settings.network.proxyTitle") }}
                </CardTitle>
            </CardHeader>
            <CardContent class="p-0">
                <div class="space-y-4">
                    <div class="space-y-2">
                        <Label>{{ t("settings.network.proxyType") }}</Label>
                        <RadioGroup v-model="config.proxy_type" class="flex flex-col gap-2">
                            <div v-for="p in proxyTypes" :key="p.value" class="flex items-center gap-2">
                                <RadioGroupItem :id="`cfg-proxy-${p.value}`" :value="p.value" class="size-4" />
                                <Label :for="`cfg-proxy-${p.value}`">{{ t(p.labelKey) }}</Label>
                            </div>
                        </RadioGroup>
                    </div>

                    <div
                        class="space-y-4"
                        :class="proxyEnabled() ? '' : 'pointer-events-none opacity-50'"
                        :aria-disabled="!proxyEnabled()"
                    >
                        <div class="grid grid-cols-1 gap-4 sm:grid-cols-[1fr_8rem]">
                            <div class="space-y-1.5">
                                <Label for="cfg-proxy-addr">{{ t("settings.network.proxyAddr") }}</Label>
                                <Input
                                    id="cfg-proxy-addr"
                                    v-model="config.proxy_addr"
                                    :class="errors.proxy_addr ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                                    :aria-invalid="!!errors.proxy_addr"
                                    :placeholder="t('settings.network.proxyAddrPlaceholder')"
                                />
                                <p v-if="errors.proxy_addr" class="text-xs text-destructive">
                                    {{ errors.proxy_addr }}
                                </p>
                            </div>
                            <div class="space-y-1.5">
                                <Label for="cfg-proxy-port">{{ t("settings.network.proxyPort") }}</Label>
                                <Input
                                    id="cfg-proxy-port"
                                    v-model="proxyPortText"
                                    inputmode="numeric"
                                    :class="errors.proxy_port || proxyPortInvalid ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                                    :aria-invalid="!!errors.proxy_port || proxyPortInvalid"
                                />
                                <p v-if="errors.proxy_port" class="text-xs text-destructive">
                                    {{ errors.proxy_port }}
                                </p>
                            </div>
                        </div>

                        <div class="flex items-center justify-between gap-4">
                            <div>
                                <Label for="cfg-proxy-auth">{{ t("settings.network.proxyAuth") }}</Label>
                                <p class="mt-0.5 text-xs text-muted-foreground">
                                    {{ t("settings.network.proxyAuthHint") }}
                                </p>
                            </div>
                            <Switch id="cfg-proxy-auth" v-model:checked="config.proxy_auth" />
                        </div>

                        <div v-if="config.proxy_auth" class="space-y-4">
                            <div class="space-y-1.5">
                                <Label for="cfg-proxy-user">{{ t("settings.network.proxyUsername") }}</Label>
                                <Input
                                    id="cfg-proxy-user"
                                    v-model="config.proxy_username"
                                    :class="errors.proxy_username ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                                    :aria-invalid="!!errors.proxy_username"
                                />
                                <p v-if="errors.proxy_username" class="text-xs text-destructive">
                                    {{ errors.proxy_username }}
                                </p>
                            </div>
                            <div class="space-y-1.5">
                                <Label for="cfg-proxy-pwd">{{ t("settings.network.proxyPassword") }}</Label>
                                <div class="relative">
                                    <Input
                                        id="cfg-proxy-pwd"
                                        v-model="config.proxy_password"
                                        :type="showPassword ? 'text' : 'password'"
                                        :autocomplete="showPassword ? 'off' : 'new-password'"
                                        class="pr-10"
                                    />
                                    <Button
                                        type="button"
                                        variant="ghost"
                                        size="icon-sm"
                                        class="absolute top-1/2 right-1 -translate-y-1/2"
                                        :aria-label="showPassword ? t('settings.network.hidePassword') : t('settings.network.showPassword')"
                                        @click="showPassword = !showPassword"
                                    >
                                        <EyeOff v-if="showPassword" class="size-4" />
                                        <Eye v-else class="size-4" />
                                    </Button>
                                </div>
                            </div>
                        </div>
                    </div>
                </div>
            </CardContent>
        </Card>

        <!-- ── 连接设置 ── -->
        <Card class="gap-0 rounded-lg p-4 shadow-none">
            <CardHeader class="mb-4 gap-0 p-0">
                <CardTitle class="text-sm font-semibold">{{ t("settings.network.connTitle") }}</CardTitle>
            </CardHeader>
            <CardContent class="p-0">
                <div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
                    <div class="space-y-1.5">
                        <Label for="cfg-api-timeout">{{ t("settings.network.apiTimeout") }}</Label>
                        <Input
                            id="cfg-api-timeout"
                            v-model="apiTimeoutText"
                            inputmode="numeric"
                            :class="errors.api_timeout_secs || apiTimeoutInvalid ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                            :aria-invalid="!!errors.api_timeout_secs || apiTimeoutInvalid"
                        />
                        <p v-if="errors.api_timeout_secs" class="text-xs text-destructive">
                            {{ errors.api_timeout_secs }}
                        </p>
                    </div>
                    <div class="space-y-1.5">
                        <Label for="cfg-stream-timeout">{{ t("settings.network.streamTimeout") }}</Label>
                        <Input
                            id="cfg-stream-timeout"
                            v-model="streamTimeoutText"
                            inputmode="numeric"
                            :class="errors.stream_timeout_secs || streamTimeoutInvalid ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                            :aria-invalid="!!errors.stream_timeout_secs || streamTimeoutInvalid"
                        />
                        <p v-if="errors.stream_timeout_secs" class="text-xs text-destructive">
                            {{ errors.stream_timeout_secs }}
                        </p>
                    </div>
                    <div class="space-y-1.5">
                        <Label for="cfg-max-retries">{{ t("settings.network.retries") }}</Label>
                        <Input
                            id="cfg-max-retries"
                            v-model="retriesText"
                            inputmode="numeric"
                            :class="errors.max_retries || retriesInvalid ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                            :aria-invalid="!!errors.max_retries || retriesInvalid"
                        />
                        <p v-if="errors.max_retries" class="text-xs text-destructive">
                            {{ errors.max_retries }}
                        </p>
                    </div>
                    <div class="space-y-1.5">
                        <Label for="cfg-retry-delay">{{ t("settings.network.retryDelay") }}</Label>
                        <Input
                            id="cfg-retry-delay"
                            v-model="retryDelayText"
                            inputmode="numeric"
                            :class="errors.retry_delay_secs || retryDelayInvalid ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                            :aria-invalid="!!errors.retry_delay_secs || retryDelayInvalid"
                        />
                        <p v-if="errors.retry_delay_secs" class="text-xs text-destructive">
                            {{ errors.retry_delay_secs }}
                        </p>
                    </div>
                </div>
            </CardContent>
        </Card>

        <!-- ── 自定义 DNS ── -->
        <Card class="gap-0 rounded-lg p-4 shadow-none">
            <CardHeader class="mb-3 gap-1 p-0">
                <CardTitle class="text-sm font-semibold">{{ t("settings.network.dnsTitle") }}</CardTitle>
                <p class="text-xs text-muted-foreground">{{ t("settings.network.dnsHint") }}</p>
            </CardHeader>
            <CardContent class="p-0">
                <div class="space-y-1.5">
                    <Label for="cfg-custom-dns">{{ t("settings.network.customDns") }}<NotEffectiveBadge /></Label>
                    <Input
                        id="cfg-custom-dns"
                        v-model="config.custom_dns"
                        :class="errors.custom_dns ? 'border-destructive focus-visible:ring-destructive/40' : ''"
                        :aria-invalid="!!errors.custom_dns"
                        :placeholder="t('settings.network.customDnsPlaceholder')"
                    />
                    <p v-if="errors.custom_dns" class="text-xs text-destructive">
                        {{ errors.custom_dns }}
                    </p>
                </div>
            </CardContent>
        </Card>
    </div>
</template>
