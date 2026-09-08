<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { computed, onMounted, ref, watch } from 'vue'
import { refreshCrossDevice, useCrossDevice } from '../../composables/useCrossDevice'
import SyncSettingsPreview from './SyncSettingsPreview.vue'

interface ConnectionConfig {
  device_name: string
  listen_ip: string
  listen_port: number
  peer_host: string
  peer_port: number
}
interface ConfigResult { config: ConnectionConfig, peer_name: string | null }
const busy = ref(false)
const loaded = ref(false)
const error = ref('')
const notice = ref('')
const peerName = ref<string | null>(null)
const pairingCode = ref('')
const ownCode = ref('')
const savedConfig = ref('')
const config = ref<ConnectionConfig>({ device_name: '', listen_ip: '', listen_port: 5540, peer_host: '', peer_port: 5540 })
const { crossState } = useCrossDevice()
const hasChanges = computed(() => JSON.stringify(config.value) !== savedConfig.value)
const incomingPeerName = computed(() => {
  try {
    const code = pairingCode.value.trim()
    if (!code.startsWith('iterate-pair-v2:'))
      return ''
    const bytes = Uint8Array.from(atob(code.slice('iterate-pair-v2:'.length)), char => char.charCodeAt(0))
    const pair = JSON.parse(new TextDecoder().decode(bytes))
    return typeof pair.name === 'string' ? pair.name.slice(0, 64) : ''
  }
  catch { return '' }
})
watch(config, () => { ownCode.value = '' }, { deep: true })
const status = computed(() => crossState.value.connected
  ? (crossState.value.peer_enabled ? '加密通道已连接，对端已开启' : '加密通道已连接，等待对端开启')
  : '对端未连接')

async function loadSettings() {
  busy.value = true
  loaded.value = false
  error.value = ''
  notice.value = ''
  pairingCode.value = ''
  ownCode.value = ''
  try {
    const result = await invoke<ConfigResult>('get_cross_device_config')
    config.value = result.config
    peerName.value = result.peer_name
    savedConfig.value = JSON.stringify(result.config)
    loaded.value = true
  }
  catch (cause) { error.value = String(cause) }
  finally { busy.value = false }
}

async function runAction(action: 'save' | 'test' | 'generate' | 'pair') {
  if (![config.value.listen_port, config.value.peer_port].every(port => Number.isInteger(port) && port >= 1 && port <= 65535)) {
    error.value = '端口须为 1–65535'
    return
  }
  busy.value = true
  error.value = ''
  notice.value = ''
  try {
    if (action !== 'test') {
      const result = await invoke<ConfigResult>('save_cross_device_config', {
        config: config.value,
        pairingCode: action === 'pair' ? pairingCode.value.trim() : null,
      })
      config.value = result.config
      peerName.value = result.peer_name
      savedConfig.value = JSON.stringify(result.config)
      ownCode.value = ''
      if (action === 'pair') {
        pairingCode.value = ''
        notice.value = `已添加对端 ${peerName.value || ''}，正在检查连接。`
      }
      else {
        notice.value = '本机地址已保存，接收服务已就绪。'
      }
    }
    if (action === 'generate') {
      ownCode.value = await invoke<string>('generate_cross_device_pairing')
      await copyCode()
    }
    else if (action === 'test' || action === 'pair') {
      const result = await invoke<{ message: string }>('test_cross_device_connection', { config: config.value, pairingCode: null })
      notice.value = result.message
    }
  }
  catch (cause) {
    error.value = String(cause)
    if (action === 'pair' && !pairingCode.value)
      notice.value = '对端资料已保存，连接检查未通过。请确认另一端也已导入本机配对码，且地址可达。'
  }
  finally {
    await refreshCrossDevice().catch(() => {})
    busy.value = false
  }
}

async function copyCode() {
  try {
    await navigator.clipboard.writeText(ownCode.value)
    notice.value = '配对码已复制，只发送给需要配对的设备。'
  }
  catch { error.value = '复制失败，请选中配对码手动复制。' }
}

onMounted(loadSettings)
</script>

<template>
  <section aria-label="跨设备配置">
    <p class="mb-3 text-sm opacity-75">
      两端分别生成并交换配对码，再各自点击“配对并连接”。IP 支持以 / 分隔填写主、备用地址；连接成功后，在标题栏开启提醒。
    </p>
    <n-form label-placement="top" size="small" :disabled="busy || !loaded" :show-feedback="false">
      <n-form-item label="本机名称" class="mb-3">
        <n-input v-model:value="config.device_name" aria-label="本机名称" placeholder="例如：办公电脑" :maxlength="64" />
      </n-form-item>
      <div class="flex gap-3 mb-3">
        <n-form-item label="本机 IP（主 / 备用 / …）" class="flex-1">
          <n-input v-model:value="config.listen_ip" aria-label="本机 IP" placeholder="192.168.1.10/10.8.0.4/公网 IP" />
        </n-form-item>
        <n-form-item label="接收端口" style="width: 110px">
          <n-input-number v-model:value="config.listen_port" aria-label="接收端口" :min="1" :max="65535" :precision="0" :show-button="false" />
        </n-form-item>
      </div>
      <p class="mb-3 text-xs opacity-75">
        本机列表包含各工作环境的网卡、VPN 或公网 IP，须包含当前网卡地址。公网地址需在路由器映射到本机接收端口；切换网络后自动使用当前可用的本机地址。
      </p>
      <div class="flex gap-3 mb-3">
        <n-form-item label="对端 IP（主 / 备用 / …）" class="flex-1">
          <n-input v-model:value="config.peer_host" aria-label="对端 IP" placeholder="192.168.1.20/10.8.0.3/公网 IP；也可导入配对码" />
        </n-form-item>
        <n-form-item label="对端端口" style="width: 110px">
          <n-input-number v-model:value="config.peer_port" aria-label="对端端口" :min="1" :max="65535" :precision="0" :show-button="false" />
        </n-form-item>
      </div>
      <p class="mb-3 text-xs opacity-75">
        按填写顺序连接，当前地址不通时尝试下一项；各地址使用同一端口，均须通向同一台已配对设备。
      </p>
      <n-form-item label="对端配对码" class="mb-3">
        <n-input v-model:value="pairingCode" type="password" show-password-on="click" aria-label="对端配对码" :placeholder="peerName ? `已添加：${peerName}；更换时粘贴新码` : '粘贴另一台设备生成的配对码'" />
      </n-form-item>
    </n-form>
    <p v-if="incomingPeerName" class="text-sm mb-3">即将添加对端：{{ incomingPeerName }}</p>
    <div class="flex gap-2 mb-3">
      <n-button size="small" :disabled="busy || !loaded" @click="runAction('generate')">
        生成并复制本机配对码
      </n-button>
      <n-button v-if="ownCode" size="small" :disabled="busy || hasChanges" @click="copyCode">
        复制配对码
      </n-button>
    </div>
    <n-input v-if="ownCode" :value="ownCode" type="password" show-password-on="click" readonly aria-label="本机配对码" class="mb-3" />
    <p class="text-xs opacity-75 mb-3">
        配对信息保存在本机 JSON 文件中，不参与设置同步。
    </p>
    <p role="status" class="mb-3">
      {{ status }}<span v-if="crossState.connected && crossState.connected_ip"> · {{ crossState.using_backup ? '备用 IP' : '主 IP' }} {{ crossState.connected_ip }}</span><span v-if="peerName"> · 已添加对端 {{ peerName }}</span><span> · 本机提醒{{ crossState.enabled ? '已开启' : '未开启' }}</span>
    </p>
    <p v-if="error" role="alert" class="mb-3 text-red-500 whitespace-pre-wrap break-all">
      {{ error }}
    </p>
    <p v-if="notice" role="status" class="mb-3 text-green-600">
      {{ notice }}
    </p>
    <div class="flex justify-end gap-2">
      <n-button v-if="peerName && !pairingCode.trim()" :loading="busy" :disabled="!loaded || hasChanges" @click="runAction('test')">
        重新检查连接
      </n-button>
      <n-button v-if="peerName && hasChanges && !pairingCode.trim()" :loading="busy" :disabled="!loaded" @click="runAction('save')">
        保存修改
      </n-button>
      <n-button type="primary" :loading="busy" :disabled="!loaded || !pairingCode.trim()" @click="runAction('pair')">
        配对并连接
      </n-button>
    </div>
    <SyncSettingsPreview :disabled="busy || !loaded || !peerName || hasChanges || !!pairingCode.trim()" :peer-name="peerName" />
  </section>
</template>
