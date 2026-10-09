<script setup lang="ts">
import { computed } from 'vue'
import { useCrossDevice } from '../../composables/useCrossDevice'

const { crossState, serverTitle } = useCrossDevice()
const phase = computed(() => crossState.value.server_connected ? 'connected' : 'disconnected')
</script>

<template>
  <span
    class="inline-flex h-8 w-8 items-center justify-center rounded-full"
    :class="phase === 'connected' ? 'text-green-600' : 'text-gray-500'"
    :title="serverTitle" :aria-label="serverTitle" role="status"
    :data-state="phase" data-testid="server-connection-status"
  >
    <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <rect x="4" y="3" width="16" height="8" rx="2" />
      <rect x="4" y="13" width="16" height="8" rx="2" />
      <path d="M8 7h.01M8 17h.01M12 7h4M12 17h4" />
    </svg>
  </span>
</template>
