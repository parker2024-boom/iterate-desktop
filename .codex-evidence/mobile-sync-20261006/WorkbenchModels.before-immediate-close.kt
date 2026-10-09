package com.kexin94yyds.iterate.mobile

enum class WorkbenchRoute {
  Conversation,
  Home,
}

enum class BridgeConnectionState {
  Connecting,
  Connected,
  Offline,
}

enum class VoiceInputState {
  Idle,
  Listening,
  PermissionDenied,
  Error,
}

enum class RequestOrigin { Mcp, Native, Mirror }
enum class SourceComputerPlatform { Windows, Mac }

data class MobileRequest(
  val requestId: String?,
  val projectPath: String?,
  val projectName: String,
  val message: String,
  val browserAiResponse: String?,
  val predefinedOptions: List<String>,
  val images: List<MobileImage> = emptyList(),
  val conversationTitle: String = "未命名弹窗",
  val isMarkdown: Boolean = true,
  val origin: RequestOrigin = RequestOrigin.Mcp,
  val timelineRouteId: String? = null,
  val conversationId: String? = null,
  val codexThreadId: String? = null,
  val codexThreadProvenance: String? = null,
  val sourceComputerId: String? = null,
  val sourceComputerName: String? = null,
  val sourceComputerPlatform: SourceComputerPlatform? = null,
)

internal val MobileRequest.kindLabel: String get() = when (origin) {
  RequestOrigin.Native -> "CLI question"
  RequestOrigin.Mcp -> "MCP"
  RequestOrigin.Mirror -> "异端"
}

internal val MobileRequest.computerSourceLabel: String get() {
  val name = sourceComputerName?.trim()?.takeIf { it.isNotEmpty() } ?: "来源设备未知"
  val id = sourceComputerId?.trim()?.takeIf { it.isNotEmpty() }
  return if (id == null) name else "$name · ${id.take(8)}"
}

data class MobileTimelineNode(
  val id: String,
  val parentId: String?,
  val role: String,
  val content: String,
  val eventId: String? = null,
)

data class MobileTimelineUpdate(
  val routeId: String,
  val projectPath: String,
  val conversationId: String,
  val nodes: List<MobileTimelineNode>,
  val snapshot: Boolean,
)

data class MobileTimeline(
  val conversationId: String,
  val nodes: List<MobileTimelineNode>,
  val position: Int,
)

data class NativeHistorySession(
  val routeId: String,
  val projectPath: String,
  val title: String,
  val launchId: String,
  val threadId: String,
)

data class MobileImage(val url: String, val alt: String)

data class QuickTemplate(val id: String, val name: String, val content: String, val sortOrder: Int)

data class QuickTemplateSnapshot(val syncReason: String, val templates: List<QuickTemplate>)

data class ActionReceipt(
  val delivered: Boolean,
  val error: String?,
  val deliveryUnknown: Boolean = false,
  val authoritativeRejection: Boolean = false,
  val resultKind: String? = null,
)

data class RequestDraft(
  val text: String,
  val selectedOptions: Set<String>,
  val deliveryUnknown: Boolean = false,
  val unconfirmedAction: UnconfirmedAction? = null,
  val requestSnapshot: MobileRequest? = null,
  val origin: RequestOrigin = RequestOrigin.Mcp,
  val originalDeviceId: String? = null,
)

data class UnconfirmedAction(
  val action: String,
  val clientActionId: String,
  val payload: String,
  val text: String,
  val selectedOptions: Set<String>,
  val origin: RequestOrigin = RequestOrigin.Mcp,
  val originalDeviceId: String? = null,
)

data class PendingAction(
  val requestId: String,
  val projectPath: String,
  val clientActionId: String,
  val action: String,
  val payload: String,
  val text: String,
  val selectedOptions: Set<String>,
  val requestSnapshot: MobileRequest? = null,
  val closeObserved: Boolean = false,
  val origin: RequestOrigin = RequestOrigin.Mcp,
  val originalDeviceId: String? = null,
  val timelineRouteAtStart: Pair<String, String>? = null,
)

private typealias RequestKey = Pair<String, String>

private fun PendingAction.key(): RequestKey = requestId to projectPath

internal fun PendingAction.matchesReceipt(requestId: String, projectPath: String, clientActionId: String): Boolean =
  this.requestId == requestId && this.projectPath == projectPath && this.clientActionId == clientActionId

private fun MobileRequest.key(): RequestKey? {
  val id = requestId?.takeIf { it.isNotBlank() } ?: return null
  if (origin == RequestOrigin.Mirror) return id to ""
  val path = projectPath?.takeIf { it.isNotBlank() } ?: return null
  return id to path
}

private fun validOptions(selected: Set<String>, request: MobileRequest): Set<String> =
  selected.filterTo(mutableSetOf()) { it in request.predefinedOptions }

internal fun WorkbenchUiState.timelineRouteFor(target: MobileRequest): Pair<String, String>? {
  if (target.origin == RequestOrigin.Mirror) return null
  val project = target.projectPath?.takeIf { it.isNotBlank() } ?: return null
  val thread = target.codexThreadId?.takeIf { it.isNotBlank() }
  if (thread != null) {
    val native = target.origin == RequestOrigin.Native
    val trusted = target.codexThreadProvenance == "caller_meta"
    if (native || trusted) {
      val matches = nativeHistorySessions.filter { it.threadId == thread &&
        (!native || (it.projectPath == project && it.routeId == target.timelineRouteId)) }
      if (matches.size == 1) return matches[0].routeId to matches[0].projectPath
    }
  }
  return target.timelineRouteId?.takeIf { it.isNotBlank() }?.let { it to project }
}

data class WorkbenchUiState(
  val route: WorkbenchRoute = WorkbenchRoute.Conversation,
  val connectionState: BridgeConnectionState = BridgeConnectionState.Connecting,
  val request: MobileRequest? = null,
  val requests: List<MobileRequest> = emptyList(),
  val timelines: Map<Pair<String, String>, MobileTimeline> = emptyMap(),
  val nativeHistorySessions: List<NativeHistorySession> = emptyList(),
  val viewedTimelineRoute: Pair<String, String>? = null,
  val historyBrowsing: Boolean = false,
  val drafts: Map<Pair<String, String>, RequestDraft> = emptyMap(),
  val closedRequestKeys: Set<Pair<String, String>> = emptySet(),
  val terminalMirrorRequestIds: Set<String> = emptySet(),
  val pendingAction: PendingAction? = null,
  val closedDraftKey: Pair<String,String>? = null,
  val queryPendingKey: Pair<String,String>? = null,
  val queryNotices: Map<Pair<String,String>,String> = emptyMap(),
  val queryConfirmedKeys: Set<Pair<String,String>> = emptySet(),
  val currentDeviceId: String? = null,
  val keepConversationCollapsed: Boolean = false,
  val paired: Boolean = false,
  val pairingBusy: Boolean = false,
  val pairingInput: String = "",
  val sending: Boolean = false,
  val actionNotice: String? = null,
  val quickTemplates: List<QuickTemplate> = emptyList(),
  val templateRefreshing: Boolean = false,
  val templateNotice: String? = null,
  val userInput: String = "",
  val selectedOptions: Set<String> = emptySet(),
  val voiceState: VoiceInputState = VoiceInputState.Idle,
  val notificationsEnabled: Boolean = true,
  val lightAppearance: Boolean = true,
  val bridgeError: String? = null,
) {
  val canSendContinue: Boolean
    get() = request?.let { current ->
      current.origin != RequestOrigin.Native && paired &&
        connectionState == BridgeConnectionState.Connected && !sending && closedDraftKey == null &&
        drafts[current.requestId to current.projectPath]?.unconfirmedAction == null
    } == true

  fun prepareContinueReply(): WorkbenchUiState? = if (canSendContinue)
    copy(userInput = "按最佳实践规范继续", selectedOptions = emptySet()) else null

  val viewedTimeline: MobileTimeline?
    get() = viewedTimelineRoute?.let(timelines::get)
  val popupTitle: String
    get() = closedDraftKey?.let { "已关闭·待核实" } ?: request?.conversationTitle ?: "待回复"

  val requestStatusText: String
    get() = when {
      closedDraftKey != null -> "原请求已关闭 · 回复待核实"
      !paired -> "请扫描电脑上的 Android 配对码"
      sending -> if (requests.isEmpty()) "正在等待送达确认" else "正在等待送达确认 · ${requests.size} 条待回复"
      request == null && requests.isNotEmpty() -> "${requests.size} 条待回复 · 点 tab 查看"
      request == null -> "暂无待回复请求"
      requests.size > 1 -> "${requests.size} 条待回复 · 点标题切换"
      else -> "待回复 · 点标题查看"
    }

  val connectionLabel: String
    get() = when (connectionState) {
      BridgeConnectionState.Connected -> "已连接"
      BridgeConnectionState.Connecting -> "连接中"
      BridgeConnectionState.Offline -> "离线"
    }
}

internal fun WorkbenchUiState.insertQuickTemplate(template: QuickTemplate): WorkbenchUiState {
  if (request == null || template !in quickTemplates) return this
  if (template.content.isBlank()) return copy(userInput = "")
  val separator = if (userInput.isEmpty() || userInput.endsWith('\n') || userInput.endsWith(' ') ||
    template.content.startsWith('\n') || template.content.startsWith(' ')) "" else "\n"
  return copy(userInput = userInput + separator + template.content)
}

internal fun WorkbenchUiState.applyQuickTemplateSnapshot(
  snapshot: QuickTemplateSnapshot,
  expectedSyncReason: String?,
): WorkbenchUiState {
  if (snapshot.syncReason != expectedSyncReason) return this
  return copy(quickTemplates = snapshot.templates, templateRefreshing = false, templateNotice = null)
}

private fun WorkbenchUiState.rememberCurrentDraft(): Map<RequestKey, RequestDraft> {
  val key = request?.key() ?: return drafts
  val previous = drafts[key]
  return drafts + (key to RequestDraft(
    text = userInput,
    selectedOptions = selectedOptions,
    deliveryUnknown = pendingAction?.key() == key || previous?.deliveryUnknown == true,
    unconfirmedAction = pendingAction?.takeIf { it.key() == key }?.let {
      UnconfirmedAction(it.action, it.clientActionId, it.payload, it.text, it.selectedOptions,it.origin,it.originalDeviceId)
    } ?: previous?.unconfirmedAction,
    requestSnapshot = request ?: previous?.requestSnapshot,
    origin = request?.origin ?: previous?.origin ?: RequestOrigin.Mcp,
    originalDeviceId = pendingAction?.takeIf {it.key()==key}?.originalDeviceId ?: previous?.originalDeviceId ?: currentDeviceId,
  ))
}

internal fun WorkbenchUiState.selectActiveRequest(target: MobileRequest): WorkbenchUiState {
  val key = target.key() ?: return this
  if (request?.key() == key) return copy(
    request = target,
    historyBrowsing = false,
    viewedTimelineRoute = timelineRouteFor(target),
    closedDraftKey = null,
    selectedOptions = validOptions(selectedOptions, target),
    keepConversationCollapsed = false,
    route = WorkbenchRoute.Conversation,
  )
  val saved = rememberCurrentDraft()
  val draft = saved[key]
  return copy(
    request = target,
    historyBrowsing = false,
    viewedTimelineRoute = timelineRouteFor(target),
    closedDraftKey = null,
    drafts = saved,
    userInput = draft?.text.orEmpty(),
    selectedOptions = validOptions(draft?.selectedOptions.orEmpty(), target),
    actionNotice = if (draft?.deliveryUnknown == true) "上一条回复是否送达未知；请先核对电脑上的原窗口" else null,
    keepConversationCollapsed = false,
    route = WorkbenchRoute.Conversation,
  )
}

internal fun WorkbenchUiState.receiveRequest(incoming: MobileRequest): WorkbenchUiState {
  val key = incoming.key() ?: return this
  if (incoming.origin == RequestOrigin.Mirror && incoming.requestId in terminalMirrorRequestIds) return this
  if (pendingAction?.key() == key) return this
  if (key in closedRequestKeys && incoming.origin != RequestOrigin.Mirror) return this
  val revisedDrafts = drafts[key]?.let { draft ->
    drafts + (key to draft.copy(selectedOptions = validOptions(draft.selectedOptions, incoming)))
  } ?: drafts
  val updated = copy(
    requests = requests.filterNot { it.key() == key } + incoming,
    closedRequestKeys = if (incoming.origin == RequestOrigin.Mirror) closedRequestKeys - key else closedRequestKeys,
    viewedTimelineRoute = if (request == null && closedDraftKey == null)
      timelineRouteFor(incoming)
    else viewedTimelineRoute,
    drafts = revisedDrafts,
  )
  val selected = request
  if (selected == null) {
    if (keepConversationCollapsed || closedDraftKey != null) return updated
    val draft = revisedDrafts[key]
    return updated.copy(
      request = incoming,
      historyBrowsing = false,
      userInput = draft?.text.orEmpty(),
      selectedOptions = draft?.selectedOptions.orEmpty(),
      actionNotice = if (draft?.deliveryUnknown == true) "上一条回复是否送达未知；请先核对电脑上的原窗口" else null,
    )
  }
  return if (selected.key() == key) updated.copy(
    request = incoming,
    viewedTimelineRoute = timelineRouteFor(incoming),
    selectedOptions = validOptions(selectedOptions, incoming),
  ) else updated
}

internal fun WorkbenchUiState.applyTimeline(update: MobileTimelineUpdate): WorkbenchUiState {
  if (update.routeId.isBlank() || update.projectPath.isBlank() || update.conversationId.isBlank()) return this
  val key = update.routeId to update.projectPath
  val previous = timelines[key]
  val allNodes = if (update.snapshot || previous?.conversationId != update.conversationId) {
    update.nodes.distinctBy { it.id }
  } else {
    (previous.nodes + update.nodes).distinctBy { it.id }
  }
  val merged = if (update.routeId.startsWith("native-cli:")) {
    val first = allNodes.firstOrNull { it.role == "user" && it.eventId?.startsWith("native-first-user:") == true }
    if (first == null) allNodes.takeLast(30) else {
      val retained = listOf(first) + allNodes.filterNot { it.id == first.id }.takeLast(29)
      retained.mapIndexed { index, node -> node.copy(parentId = retained.getOrNull(index - 1)?.id) }
    }
  } else allNodes.takeLast(30)
  val wasAtEnd = previous == null || previous.position >= previous.nodes.lastIndex
  val position = if (merged.isEmpty()) 0 else if (wasAtEnd) merged.lastIndex
    else merged.indexOfFirst { it.id == previous!!.nodes.getOrNull(previous.position)?.id }
      .takeIf { it >= 0 } ?: 0
  val current = request
  val selectedKey = current?.timelineRouteId?.let { it to current.projectPath.orEmpty() }
  return copy(
    timelines = timelines + (key to MobileTimeline(update.conversationId, merged, position)),
    viewedTimelineRoute = if (viewedTimelineRoute == null && selectedKey == key) key else viewedTimelineRoute,
  )
}

internal fun WorkbenchUiState.moveTimeline(position: Int): WorkbenchUiState {
  val key = viewedTimelineRoute ?: return this
  val timeline = timelines[key] ?: return this
  if (timeline.nodes.isEmpty()) return this
  return copy(historyBrowsing = true,
    timelines = timelines + (key to timeline.copy(position = position.coerceIn(0, timeline.nodes.lastIndex))))
}

internal fun WorkbenchUiState.selectNativeHistory(session: NativeHistorySession): WorkbenchUiState =
  copy(viewedTimelineRoute = session.routeId to session.projectPath, historyBrowsing = true,
    route = WorkbenchRoute.Conversation)

internal fun WorkbenchUiState.beginAction(requestId: String, projectPath: String, clientActionId: String,
  action: String = "submit", payload: String = "", deviceId: String? = null): WorkbenchUiState {
  val key = requestId to projectPath
  return copy(
    route = WorkbenchRoute.Conversation,
    keepConversationCollapsed = true,
    request = if(request?.origin==RequestOrigin.Native) request else null,
    requests = if(request?.origin==RequestOrigin.Native) requests else requests.filterNot { it.key() == key },
    drafts = rememberCurrentDraft(),
    userInput = if(request?.origin==RequestOrigin.Native) userInput else "",
    selectedOptions = if(request?.origin==RequestOrigin.Native) selectedOptions else emptySet(),
    sending = true,
    pendingAction = PendingAction(requestId, projectPath, clientActionId, action, payload, userInput, selectedOptions,
      requestSnapshot = request?.takeIf { it.key() == (requestId to projectPath) }, origin=request?.origin ?: RequestOrigin.Mcp,originalDeviceId=deviceId,
      timelineRouteAtStart = request?.takeIf { it.key() == key }?.let { timelineRouteFor(it) }),
    actionNotice = null,
  )
}

internal fun WorkbenchUiState.closeRequest(requestId: String, projectPath: String): WorkbenchUiState {
  val key = requestId to projectPath
  val pendingClosing = pendingAction?.takeIf { it.key() == key }
  val closing = request?.takeIf { it.key() == key } ?: requests.firstOrNull { it.key() == key }
    ?: pendingClosing?.requestSnapshot
  val closingRoute = if (pendingClosing?.requestSnapshot != null) pendingClosing.timelineRouteAtStart
    else closing?.let { timelineRouteFor(it) }
  val closingHistoryVisible = closingRoute != null && closingRoute == viewedTimelineRoute
  val remaining = requests.filterNot { it.key() == key }
  val selectedWasClosed = request?.key() == key
  val closeIsPending = pendingAction?.let {
    it.requestId == requestId && it.projectPath == projectPath
  } == true
  val savedDrafts = rememberCurrentDraft()
  val closingDraft = savedDrafts[key]
  // Closing the source is not a delivery receipt. Keep unsent and uncertain
  // input until an exact delivered receipt clears this request's draft.
  val keptDrafts = if (closeIsPending || (closingDraft != null &&
    (closingDraft.deliveryUnknown || closingDraft.unconfirmedAction != null ||
      closingDraft.text.isNotEmpty() || closingDraft.selectedOptions.isNotEmpty()))) savedDrafts
    else savedDrafts - key
  val next = if (selectedWasClosed) remaining.firstOrNull() else request
  val nextDraft = next?.key()?.let(keptDrafts::get)
  return copy(
    requests = remaining,
    request = next,
    viewedTimelineRoute = if (selectedWasClosed || closingHistoryVisible) next?.let { timelineRouteFor(it) } else viewedTimelineRoute,
    historyBrowsing = if (selectedWasClosed || closingHistoryVisible) false else historyBrowsing,
    drafts = keptDrafts,
    closedRequestKeys = closedRequestKeys + key,
    pendingAction = if (closeIsPending) pendingAction?.copy(closeObserved = true) else pendingAction,
    userInput = if (selectedWasClosed) nextDraft?.text.orEmpty() else userInput,
    selectedOptions = if (selectedWasClosed) nextDraft?.selectedOptions.orEmpty() else selectedOptions,
    actionNotice = if (closeIsPending) "电脑上的请求已关闭，正在等待送达确认" else if (selectedWasClosed) "电脑上的请求已关闭" else actionNotice,
  )
}

internal fun WorkbenchUiState.deliveredRequest(requestId: String, projectPath: String, resultKind: String? = null): WorkbenchUiState {
  val key = requestId to projectPath
  val pendingClosing = pendingAction?.takeIf { it.key() == key }
  val closing = request?.takeIf { it.key() == key } ?: requests.firstOrNull { it.key() == key }
    ?: pendingClosing?.requestSnapshot
  val closingRoute = if (pendingClosing?.requestSnapshot != null) pendingClosing.timelineRouteAtStart
    else closing?.let { timelineRouteFor(it) }
  val closingHistoryVisible = closingRoute != null && closingRoute == viewedTimelineRoute
  val remaining = requests.filterNot { it.key() == key }
  val selectedWasDelivered = request?.key() == key
  val keptDrafts = drafts - key
  val next = if (selectedWasDelivered) remaining.firstOrNull() else request
  val nextDraft = next?.key()?.let(keptDrafts::get)
  return copy(
    route = WorkbenchRoute.Conversation,
    requests = remaining,
    request = next,
    viewedTimelineRoute = if (selectedWasDelivered || closingHistoryVisible) next?.let { timelineRouteFor(it) } else viewedTimelineRoute,
    historyBrowsing = if (selectedWasDelivered || closingHistoryVisible) false else historyBrowsing,
    drafts = keptDrafts,
    userInput = if (selectedWasDelivered) nextDraft?.text.orEmpty() else userInput,
    selectedOptions = if (selectedWasDelivered) nextDraft?.selectedOptions.orEmpty() else selectedOptions,
    sending = if(pendingAction!=null&&pendingAction.key()!=key)sending else false,
    pendingAction = pendingAction?.takeIf{it.key()!=key},
    closedRequestKeys = closedRequestKeys + key,
    terminalMirrorRequestIds = if (resultKind in setOf("mac_accepted", "mirror_closed")) terminalMirrorRequestIds + requestId else terminalMirrorRequestIds,
    actionNotice = when(resultKind){"saved"->"电脑已保存";"cli_received"->"CLI 已接收";"mac_accepted"->"Mac 已接收这条回复";"mirror_closed"->"手机已隐藏该镜像，Mac 请求继续等待";else->"电脑已收到这条回复"},
  )
}

internal fun WorkbenchUiState.actionFailed(
  requestId: String,
  projectPath: String,
  receipt: ActionReceipt,
): WorkbenchUiState {
  val key = requestId to projectPath
  val savedDrafts = rememberCurrentDraft()
  val existing = savedDrafts[key]
  val pending = pendingAction?.takeIf { it.key() == key }
  val closed = pending?.closeObserved == true || key in closedRequestKeys
  val target = if(closed) null else requests.firstOrNull { it.key() == key } ?: pending?.requestSnapshot
    ?: request?.takeIf { it.key() == key }
  val originalText = pending?.text ?: if (request?.key() == key) userInput else existing?.text.orEmpty()
  val originalOptions = pending?.selectedOptions ?: if (request?.key() == key) selectedOptions else existing?.selectedOptions.orEmpty()
  val updatedDrafts = savedDrafts + (key to RequestDraft(
    text = originalText,
    selectedOptions = originalOptions,
    deliveryUnknown = if(receipt.authoritativeRejection) false else receipt.deliveryUnknown || existing?.deliveryUnknown == true,
    unconfirmedAction = if(receipt.authoritativeRejection && !closed) null else if (receipt.deliveryUnknown) pending?.let {
      UnconfirmedAction(it.action, it.clientActionId, it.payload, it.text, it.selectedOptions,it.origin,it.originalDeviceId)
    } else existing?.unconfirmedAction,
    requestSnapshot = pending?.requestSnapshot ?: existing?.requestSnapshot ?: target,
    origin = pending?.origin ?: existing?.origin ?: RequestOrigin.Mcp,
    originalDeviceId = pending?.originalDeviceId ?: existing?.originalDeviceId,
  ))
  val closeObserved = closed
  val notice = if (closeObserved) {
    "电脑上的请求已关闭，回复${if (receipt.deliveryUnknown) "送达状态未知" else "未被确认"}；请核对原窗口"
  } else receipt.error ?: "回复未送达"
  return copy(
    route = if (target != null) WorkbenchRoute.Conversation else route,
    keepConversationCollapsed = false,
    request = target ?: request,
    requests = if (target != null && requests.none { it.key() == key }) listOf(target) + requests else requests,
    sending = false,
    pendingAction = null,
    drafts = updatedDrafts,
    closedRequestKeys = if(closed) closedRequestKeys + key else closedRequestKeys - key,
    userInput = if (target != null) originalText else userInput,
    selectedOptions = if (target != null) validOptions(originalOptions, target) else selectedOptions,
    actionNotice = notice,
  )
}

internal fun WorkbenchUiState.clearRemoteRequests(
  connection: BridgeConnectionState,
  error: String? = null,
): WorkbenchUiState = copy(
  connectionState = connection,
  bridgeError = error,
  templateRefreshing = false,
  request = null,
  requests = emptyList(),
  viewedTimelineRoute = null,
  historyBrowsing = false,
  drafts = rememberCurrentDraft().let { saved ->
    pendingAction?.let { pending ->
      val key = pending.key()
      saved + (key to RequestDraft(pending.text, pending.selectedOptions, true,
        UnconfirmedAction(pending.action, pending.clientActionId, pending.payload, pending.text, pending.selectedOptions,pending.origin,pending.originalDeviceId),pending.requestSnapshot,pending.origin,pending.originalDeviceId))
    } ?: saved
  },
  userInput = "",
  selectedOptions = emptySet(),
  // The receive socket is independent of the action socket. Keep the action
  // in flight until its own exact ACK (or timeout) settles it.
  sending = sending && pendingAction != null,
  actionNotice = if (sending && pendingAction != null) "接收连接断开，正在等待手机回复送达确认" else actionNotice,
)

internal fun WorkbenchUiState.selectClosedDraft(key: Pair<String,String>): WorkbenchUiState {
  val draft=drafts[key] ?: return this
  if(key !in closedRequestKeys || draft.unconfirmedAction==null) return this
  return copy(drafts=rememberCurrentDraft(),closedDraftKey=key,route=WorkbenchRoute.Conversation)
}
internal fun WorkbenchUiState.queryDisabledReason(key: Pair<String,String>): String? = when {
  !paired -> "请先配对电脑"
  connectionState!=BridgeConnectionState.Connected -> "连接电脑后才能查询原回复"
  drafts[key]?.originalDeviceId!=currentDeviceId -> "请使用发送原回复的配对设备查询"
  queryPendingKey!=null -> "查询处理中"
  drafts[key]?.unconfirmedAction==null -> "没有可查询的原回复"
  else -> null
}
