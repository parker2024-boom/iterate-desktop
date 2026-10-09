package com.kexin94yyds.iterate.mobile

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import org.json.JSONObject
import java.io.IOException
import java.io.ByteArrayOutputStream
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.TimeUnit
import java.util.UUID

sealed interface BridgeEvent {
  data class Connected(val templateSyncReason: String) : BridgeEvent
  data class Request(val request: MobileRequest) : BridgeEvent
  data class RequestClosed(val requestId: String, val projectPath: String) : BridgeEvent
  data class QuickTemplates(val snapshot: QuickTemplateSnapshot) : BridgeEvent
  data class ActionResult(val text:String):BridgeEvent
  data class Timeline(val update: MobileTimelineUpdate):BridgeEvent
  data class NativeHistories(val sessions: List<NativeHistorySession>):BridgeEvent
  data class Disconnected(val reason: String) : BridgeEvent
}

class BridgeClient(
  private val client: OkHttpClient = OkHttpClient.Builder()
    .connectTimeout(10, TimeUnit.SECONDS)
    .pingInterval(25, TimeUnit.SECONDS)
    .build(),
) {
  @Volatile private var receiveSocket: WebSocket? = null

  fun requestQuickTemplateSync(syncReason: String): Boolean = receiveSocket?.send(
    JSONObject().put("message_type", "request_sync")
      .put("payload", JSONObject().put("sync_reason", syncReason)).toString()
  ) == true

  fun requestTimeline(routeId: String, projectPath: String): Boolean = receiveSocket?.send(
    JSONObject().put("message_type", "request_timeline_sync")
      .put("payload", JSONObject().put("request_id", routeId).put("project_path", projectPath))
      .toString()
  ) == true

  suspend fun loadImage(pairing: SavedPairing, source: String): ByteArray = withContext(Dispatchers.IO) {
    val bridgeImage = source.matches(Regex("""/image\?id=img_[a-fA-F0-9]{32}"""))
    if (!bridgeImage && !source.startsWith("https://")) throw IOException("不支持的图片地址")
    val url = if (bridgeImage) pairing.baseUrl.trimEnd('/') + source else source
    val request = if (bridgeImage) authenticatedRequest(url, pairing).build()
      else Request.Builder().url(url).build()
    // Do not let an image redirect carry paired-device credentials to another host.
    client.newBuilder().followRedirects(false).followSslRedirects(false).build()
      .newCall(request).execute().use { response ->
        if (!response.isSuccessful) throw IOException("图片读取失败 (${response.code})")
        if (!response.header("Content-Type").orEmpty().startsWith("image/", ignoreCase = true)) {
          throw IOException("图片格式无效")
        }
        val stream = response.body?.byteStream() ?: throw IOException("图片内容为空")
        val output = ByteArrayOutputStream()
        val buffer = ByteArray(8192)
        val maximum = 8 * 1024 * 1024
        while (true) {
          val count = stream.read(buffer)
          if (count < 0) break
          if (output.size() + count > maximum) throw IOException("图片超过 8 MB")
          output.write(buffer, 0, count)
        }
        output.toByteArray()
      }
  }

  fun requests(pairing: SavedPairing): Flow<BridgeEvent> = callbackFlow {
    val request = authenticatedRequest(pairing.wsUrl, pairing).build()
    val flowClosed = AtomicBoolean(false)
    val initialSyncReason = "android_connect:${UUID.randomUUID()}"
    val socket = client.newWebSocket(request, object : WebSocketListener() {
      override fun onOpen(webSocket: WebSocket, response: Response) {
        if (flowClosed.get()) {
          webSocket.close(1000, "android_screen_closed")
          return
        }
        receiveSocket = webSocket
        trySend(BridgeEvent.Connected(initialSyncReason))
        webSocket.send(JSONObject()
          .put("message_type", "client_hello")
          .put("payload", JSONObject()
            .put("client_kind", "android")
            .put("device_id", pairing.deviceId)
            .put("selected_transport_mode", "public_tunnel")
            .put("selected_ws_url", pairing.wsUrl))
          .toString())
        webSocket.send(JSONObject().put("message_type", "request_sync")
          .put("payload", JSONObject().put("sync_reason", initialSyncReason)).toString())
      }

      override fun onMessage(webSocket: WebSocket, text: String) {
        if(runCatching{JSONObject(text).optString("message_type") in listOf("mcp_action_result", "cross_device_mirror_action_result")}.getOrDefault(false)){trySend(BridgeEvent.ActionResult(text));return}
        BridgeJsonParser.parseNativeHistorySessions(text)?.let { trySend(BridgeEvent.NativeHistories(it)); return }
        BridgeJsonParser.parseTimeline(text)?.let { trySend(BridgeEvent.Timeline(it)) }
        BridgeJsonParser.parseQuickTemplates(text)?.let { trySend(BridgeEvent.QuickTemplates(it)) }
        BridgeJsonParser.parseClosedRequest(text)?.let {
          trySend(BridgeEvent.RequestClosed(it.first, it.second))
          return
        }
        BridgeJsonParser.parseRequest(text)?.let { trySend(BridgeEvent.Request(it)) }
      }

      override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
        if (receiveSocket === webSocket) receiveSocket = null
        trySend(BridgeEvent.Disconnected(if (response?.code == 401) "配对凭据已失效" else "连接失败：${response?.code ?: t.javaClass.simpleName}"))
        close()
      }

      override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
        if (receiveSocket === webSocket) receiveSocket = null
        trySend(BridgeEvent.Disconnected("连接已断开 ($code)"))
        close()
      }
    })
    awaitClose {
      flowClosed.set(true)
      if (receiveSocket === socket) receiveSocket = null
      socket.close(1000, "android_screen_closed")
    }
  }

  suspend fun claim(payload: AndroidPairingPayload, deviceId: String): SavedPairing = withContext(Dispatchers.IO) {
    val body = JSONObject()
      .put("client_kind", "android")
      .put("pairing_token", payload.pairingToken)
      .put("device_id", deviceId)
      .put("device_name", "Android")
      .toString().toRequestBody("application/json".toMediaType())
    val request = Request.Builder().url("${payload.baseUrl}/api/android/pairing/claim").post(body).build()
    client.newCall(request).execute().use { response ->
      if (!response.isSuccessful) throw IOException("配对失败 (${response.code})")
      val json = JSONObject(response.body?.string().orEmpty())
      if (!json.optBoolean("ok") || json.optString("device_id") != deviceId) {
        throw IOException("配对回执不匹配")
      }
      val token = json.getString("device_token")
      if (!token.startsWith("dt_") || token.length < 20) throw IOException("配对凭据无效")
      SavedPairing(payload.baseUrl, payload.wsUrl, deviceId, token)
    }
  }

  suspend fun checkStatus(pairing: SavedPairing): Boolean = withContext(Dispatchers.IO) {
    val request = authenticatedRequest("${pairing.baseUrl}/api/android/pairing/status", pairing).build()
    client.newCall(request).execute().use { it.isSuccessful }
  }

  suspend fun sendAction(pairing: SavedPairing, json: String, requestId: String, projectPath: String, actionId: String, action: String, origin: RequestOrigin = RequestOrigin.Mcp): ActionReceipt = withContext(Dispatchers.IO) {
    val result = CompletableDeferred<ActionReceipt>()
    val submitted = AtomicBoolean(false)
    val request = authenticatedRequest(pairing.wsUrl, pairing).build()
    val socket = client.newWebSocket(request, object : WebSocketListener() {
      override fun onOpen(webSocket: WebSocket, response: Response) {
        if (webSocket.send(json)) submitted.set(true)
        else result.complete(ActionReceipt(false, "消息未发出"))
      }

      override fun onMessage(webSocket: WebSocket, text: String) {
        val receipt = BridgeJsonParser.parseActionReceipt(text, requestId, projectPath, actionId, action, origin)
        if (receipt != null) result.complete(receipt)
      }

      override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
        val unknown = submitted.get()
        result.complete(ActionReceipt(
          false,
          if (unknown) "送达状态未知；请先核对电脑上的原窗口" else if (response?.code == 401) "配对凭据已失效" else "连接失败",
          unknown,
        ))
      }

      override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
        val unknown = submitted.get()
        result.complete(ActionReceipt(false, if (unknown) "送达状态未知；请先核对电脑上的原窗口" else "确认前连接已关闭", unknown))
      }
    })
    try {
      withTimeout(15_000) { result.await() }
    } catch (_: Exception) {
      val unknown = submitted.get()
      ActionReceipt(false, if (unknown) "等待确认超时，送达状态未知；请先核对电脑上的原窗口" else "连接超时，消息未发出", unknown)
    } finally {
      socket.close(1000, "android_action_finished")
    }
  }

  suspend fun openCodexChat(pairing: SavedPairing, projectPath: String?): Boolean = withContext(Dispatchers.IO) {
    val body = JSONObject().apply { if (!projectPath.isNullOrBlank()) put("project_path", projectPath) }
      .toString().toRequestBody("application/json".toMediaType())
    val request = authenticatedRequest("${pairing.baseUrl}/api/open-codex-chat", pairing).post(body).build()
    runCatching { client.newCall(request).execute().use { it.isSuccessful && JSONObject(it.body?.string().orEmpty()).optBoolean("ok") } }.getOrDefault(false)
  }


  private fun authenticatedRequest(url: String, pairing: SavedPairing): Request.Builder =
    Request.Builder().url(url)
      .header("Authorization", "Bearer ${pairing.deviceToken}")
      .header("X-Iterate-Device-Id", pairing.deviceId)
      .header("X-Iterate-Client-Kind", "android")
}
