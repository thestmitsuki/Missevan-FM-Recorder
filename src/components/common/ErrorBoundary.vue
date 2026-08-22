<script setup lang="ts">
import { ref, onErrorCaptured } from "vue";
import { useI18n } from "vue-i18n";
import { Button } from "@/components/ui/button";

const { t } = useI18n();

const hasError = ref(false);
const errorMessage = ref("");
const errorKey = ref(0);

onErrorCaptured((err) => {
  hasError.value = true;
  errorMessage.value = err instanceof Error ? err.message : String(err);
  return false;
});

function resetError() {
  hasError.value = false;
  errorMessage.value = "";
  errorKey.value++;
}
</script>

<template>
  <div v-if="hasError" class="error-boundary">
    <p class="error-title">{{ t("error.boundaryTitle") }}</p>
    <p class="error-message">{{ errorMessage }}</p>
    <Button variant="outline" @click="resetError">{{ t("error.retry") }}</Button>
  </div>
  <slot v-else :key="errorKey" />
</template>

<style scoped>
.error-boundary {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 100vh;
  gap: 1rem;
  padding: 2rem;
  text-align: center;
}
.error-title {
  font-size: 1.25rem;
  font-weight: 600;
  color: var(--md-sys-color-error);
}
.error-message {
  font-size: 0.875rem;
  color: var(--md-sys-color-on-surface-variant);
  max-width: 400px;
}
</style>
