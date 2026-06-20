import { ref, onMounted, onUnmounted } from 'vue'

/** 窗口尺寸状态 */
export function useResponsive() {
  const windowWidth = ref(window.innerWidth)
  const windowHeight = ref(window.innerHeight)

  /** 当前是移动布局 (< 480px) */
  const isMobile = ref(windowWidth.value < 480)
  /** 当前是平板布局 (480-768px) */
  const isTablet = ref(windowWidth.value >= 480 && windowWidth.value < 768)
  /** 当前是桌面布局 (>= 768px) */
  const isDesktop = ref(windowWidth.value >= 768)

  function onResize() {
    windowWidth.value = window.innerWidth
    windowHeight.value = window.innerHeight
    isMobile.value = windowWidth.value < 480
    isTablet.value = windowWidth.value >= 480 && windowWidth.value < 768
    isDesktop.value = windowWidth.value >= 768
  }

  onMounted(() => window.addEventListener('resize', onResize))
  onUnmounted(() => window.removeEventListener('resize', onResize))

  return {
    windowWidth,
    windowHeight,
    isMobile,
    isTablet,
    isDesktop,
  }
}
