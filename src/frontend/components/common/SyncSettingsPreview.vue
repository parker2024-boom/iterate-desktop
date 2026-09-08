<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { computed, ref, watch } from 'vue'

const props = defineProps<{ disabled: boolean, peerName: string | null }>()
const categories = [
  ['appearance', '外观与字体'], ['window', '窗口尺寸与置顶'], ['audio', '提醒声音开关'],
  ['shortcuts', '快捷键'], ['reply', '回复提示词'], ['prompt_templates', '提示词模板'],
  ['prompt_library', '提示词库'], ['ghost_suggestions', '幽灵补全词库'],
  ['speech_replacements', '语音替换规则'], ['speech_corrections', '语音纠错规则'],
  ['speech_vocabulary', '语音词汇'], ['auto_continue', '自动继续'],
  ['clipboard_backup', '剪贴板备份'], ['mcp_tools', 'MCP 工具开关'], ['auto_checkpoint', '自动检查点'],
] as const
interface Difference { category: string, local: unknown, incoming: unknown, conflicts: unknown[] }
interface Preview {
  preview_id: string
  snapshot: { device_name: string, platform: string }
  differences: Difference[]
  warnings: string[]
}
interface Result { category: string, success: boolean, message: string }
const expanded = ref(false)
const selected = ref<string[]>(['appearance'])
const preview = ref<Preview | null>(null)
const acknowledged = ref(false)
const busy = ref(false)
const error = ref('')
const results = ref<Result[]>([])
const canPreview = computed(() => !props.disabled && !busy.value && selected.value.length > 0)
const name = (id: string) => categories.find(([key]) => key === id)?.[1] || id
const pretty = (value: unknown) => JSON.stringify(value, null, 2)

watch(selected, () => { preview.value = null; acknowledged.value = false; results.value = [] }, { deep: true })
watch(() => props.peerName, () => { preview.value = null; acknowledged.value = false })

async function loadPreview() {
  busy.value = true
  error.value = ''
  preview.value = null
  results.value = []
  acknowledged.value = false
  try { preview.value = await invoke<Preview>('settings_sync_preview', { categories: selected.value }) }
  catch (cause) { error.value = String(cause) }
  finally { busy.value = false }
}

async function apply() {
  if (!preview.value || !acknowledged.value || busy.value)
    return
  busy.value = true
  error.value = ''
  try {
    results.value = await invoke<Result[]>('settings_sync_apply', { previewId: preview.value.preview_id })
    window.dispatchEvent(new CustomEvent('iterate:settings-synced', { detail: results.value }))
  }
  catch (cause) { error.value = String(cause) }
  finally { preview.value = null; acknowledged.value = false; busy.value = false }
}
</script>

<template>
  <div class="mt-5 pt-4 border-t border-current/15">
    <n-button :disabled="busy" @click="expanded = !expanded">
      {{ expanded ? '收起设置复制' : '从对端复制设置' }}
    </n-button>
    <div v-if="expanded" class="mt-3">
      <p class="mb-3 text-sm">
        将 {{ peerName || '已配对设备' }} 的勾选设置复制到本机。IP、端口、设备名称、配对信息、账户凭据、本机路径、历史和统计保留本机。
      </p>
      <n-checkbox-group v-model:value="selected" :disabled="busy">
        <div class="grid grid-cols-2 gap-2 mb-3">
          <n-checkbox v-for="[key, label] in categories" :key="key" :value="key" :label="label" />
        </div>
      </n-checkbox-group>
      <p v-if="disabled" class="mb-3 text-sm">请先添加对端并保存地址修改，再预览对端设置。</p>
      <n-button :disabled="!canPreview" :loading="busy" @click="loadPreview">预览勾选设置</n-button>
      <div v-if="preview" class="mt-3">
        <p class="mb-2">来源：{{ preview.snapshot.device_name }}（{{ preview.snapshot.platform }}）</p>
        <p v-if="!preview.differences.length" class="mb-3">勾选设置与本机一致，无需复制。</p>
        <details v-for="difference in preview.differences" :key="difference.category" class="mb-3">
          <summary class="cursor-pointer">{{ name(difference.category) }} · 查看本机与对端差异</summary>
          <div class="grid grid-cols-2 gap-2 text-xs mt-2">
            <div>本机<pre class="whitespace-pre-wrap break-all max-h-64 overflow-auto">{{ pretty(difference.local) }}</pre></div>
            <div>对端<pre class="whitespace-pre-wrap break-all max-h-64 overflow-auto">{{ pretty(difference.incoming) }}</pre></div>
          </div>
          <p v-if="difference.conflicts.length" class="text-amber-600">{{ difference.conflicts.length }} 条冲突保留本机内容。</p>
        </details>
        <div role="note" class="text-sm text-amber-600 mb-3">
          <p v-for="warning in preview.warnings" :key="warning" class="mb-1">{{ warning }}</p>
          <p>按类别分别保存，失败类别不会覆盖本机；不会重启正在进行的任务。</p>
        </div>
        <n-checkbox v-if="preview.differences.length" v-model:checked="acknowledged" :disabled="busy" class="mb-3">
          我已检查差异和警告，确认将勾选设置复制到本机
        </n-checkbox>
        <div><n-button type="primary" :loading="busy" :disabled="disabled || !acknowledged || !preview.differences.length" @click="apply">确认复制勾选设置</n-button></div>
      </div>
      <p v-if="error" role="alert" class="mt-3 text-red-500 whitespace-pre-wrap">{{ error }}</p>
      <ul v-if="results.length" role="status" class="mt-3">
        <li v-for="result in results" :key="result.category" :class="result.success ? 'text-green-600' : 'text-red-500'">
          {{ name(result.category) }}：{{ result.message }}
        </li>
      </ul>
    </div>
  </div>
</template>
