<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { useMessage } from 'naive-ui'
import { computed, onMounted, ref } from 'vue'
import { useCrossDevice } from '../../composables/useCrossDevice'
import SyncSettingsPreview from './SyncSettingsPreview.vue'

const message = useMessage()
const busy = ref(false)
const endpoint = ref<string | null>(null)
const error = ref('')
const notice = ref('')
const { crossState, crossBusy, serverTitle, toggleCrossDevice } = useCrossDevice()
const available = computed(() => !!endpoint.value && crossState.value.connected)

onMounted(async () => {
  try {
    const result = await invoke<{ endpoint?: string | null }>('get_cross_device_config')
    endpoint.value = result.endpoint || null
  }
  catch (cause) { error.value = String(cause) }
})

async function refreshWindows() {
  if (busy.value)
    return
  busy.value = true
  error.value = ''
  notice.value = ''
  try {
    const result = await invoke<{ count: number }>('sync_cross_device_windows')
    notice.value = result.count ? `已同步 ${result.count} 个窗口，已有窗口不会重复打开。` : '没有可同步的待回复窗口。'
    message.success(notice.value)
  }
  catch (cause) { error.value = String(cause) }
  finally { busy.value = false }
}
</script>

<template>
  <section aria-label="公网同步设置">
    <div class="mb-3 flex items-center gap-2">
      <span>窗口同步</span>
      <n-switch :value="crossState.enabled" :loading="crossBusy" @update:value="toggleCrossDevice" />
    </div>
    <p role="status" class="mb-3">
      {{ serverTitle }}
    </p>
    <n-form label-placement="top" size="small" :show-feedback="false">
      <n-form-item label="公网服务" class="mb-3">
        <span>{{ endpoint || '尚未配置公网服务' }}</span>
      </n-form-item>
    </n-form>
    <p class="mb-3 text-sm opacity-75">
      通过公网自动同步待回复窗口；完成或关闭后，同一请求的窗口一起清除。
    </p>
    <div class="flex flex-wrap items-center gap-2">
      <n-button :loading="busy" :disabled="busy || !available" @click="refreshWindows">
        刷新窗口
      </n-button>
      <SyncSettingsPreview :disabled="busy || !available" :peer-name="null" />
    </div>
    <p v-if="error" role="alert" class="mt-3 text-red-500 whitespace-pre-wrap break-all">
      {{ error }}
    </p>
    <p v-if="notice" role="status" class="mt-3 text-green-600">
      {{ notice }}
    </p>
  </section>
</template>
