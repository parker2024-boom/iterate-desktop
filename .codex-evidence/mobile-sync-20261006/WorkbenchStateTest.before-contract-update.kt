package com.kexin94yyds.iterate.mobile

import org.junit.Assert.assertEquals
import org.junit.Test

class WorkbenchStateTest {
  @Test
  fun olderTemplateSyncCannotEndManualRefreshOrRollBackNewerTemplates() {
    val old = QuickTemplate("old", "旧", "旧", 1)
    val fresh = QuickTemplate("new", "新", "新", 1)
    val waiting = WorkbenchUiState(quickTemplates = listOf(old), templateRefreshing = true)
    val initialReply = QuickTemplateSnapshot("android_connect:initial-1", listOf(old))
    val manualReply = QuickTemplateSnapshot("android_template_refresh:manual-2", listOf(fresh))
    assertEquals(waiting, waiting.applyQuickTemplateSnapshot(initialReply, manualReply.syncReason))
    val updated = waiting.applyQuickTemplateSnapshot(manualReply, manualReply.syncReason)
    assertEquals(listOf(fresh), updated.quickTemplates)
    assertEquals(false, updated.templateRefreshing)
    assertEquals(updated, updated.applyQuickTemplateSnapshot(initialReply, manualReply.syncReason))
  }

  @Test
  fun templateClickOnlyInsertsIntoCurrentDraftUsingDesktopSeparatorRules() {
    val first = request("req-1", "E:\\project")
    val second = request("req-2", "E:\\project")
    val append = QuickTemplate("append", "追加", "模板内容", 1)
    val clear = QuickTemplate("clear", "清空", "  ", 2)
    val state = WorkbenchUiState(request = first, requests = listOf(first, second),
      userInput = "已有内容", quickTemplates = listOf(append, clear))
    val inserted = state.insertQuickTemplate(append)
    assertEquals("已有内容\n模板内容", inserted.userInput)
    assertEquals(first, inserted.request)
    assertEquals("已有内容\n模板内容", inserted.selectActiveRequest(second).selectActiveRequest(first).userInput)
    assertEquals("", inserted.insertQuickTemplate(clear).userInput)
    assertEquals(state, state.insertQuickTemplate(QuickTemplate("unknown", "未知", "不得插入", 3)))
    assertEquals("已有内容", state.copy(request = null).insertQuickTemplate(append).userInput)
  }

  private fun request(id: String, path: String, options: List<String> = listOf("继续")) =
    MobileRequest(id, path, path, "问题 $id", null, options)

  @Test
  fun firstRequestOpensButRequestsAfterSendStayInTabsUntilSelected() {
    val first = request("req-1", "E:\\project")
    val second = request("req-2", "E:\\project")
    val opened = WorkbenchUiState(paired = true).receiveRequest(first)
    assertEquals(first, opened.request)

    val collapsed = opened.beginAction("req-1", "E:\\project", "action-1")
    assertEquals(null, collapsed.request)
    assertEquals(emptyList<MobileRequest>(), collapsed.requests)
    assertEquals(WorkbenchRoute.Conversation, collapsed.route)

    val waiting = collapsed.receiveRequest(second)
    assertEquals(null, waiting.request)
    assertEquals(listOf(second), waiting.requests)
    assertEquals("正在等待送达确认 · 1 条待回复", waiting.requestStatusText)

    val delivered = waiting.deliveredRequest("req-1", "E:\\project")
    assertEquals(null, delivered.request)
    assertEquals(listOf(second), delivered.requests)
    assertEquals("1 条待回复 · 点 tab 查看", delivered.requestStatusText)
    assertEquals(second, delivered.selectActiveRequest(second).request)
  }

  @Test
  fun closingOneRequestKeepsOtherWindowsAndTheirDraft() {
    val first = request("req-1", "E:\\project")
    val second = request("req-2", "E:\\project")
    val third = request("req-1", "E:\\other")
    val initial = WorkbenchUiState(
      request = second,
      requests = listOf(first, second, third),
      userInput = "第二个窗口的回复",
    )
    val afterOtherWindow = initial.closeRequest("req-1", "E:\\project")
    assertEquals(listOf(second, third), afterOtherWindow.requests)
    assertEquals(second, afterOtherWindow.request)
    assertEquals("第二个窗口的回复", afterOtherWindow.userInput)

    val afterSelected = afterOtherWindow.closeRequest("req-2", "E:\\project")
    assertEquals(listOf(third), afterSelected.requests)
    assertEquals(third, afterSelected.request)
    assertEquals("", afterSelected.userInput)
  }

  @Test
  fun disconnectDropsAllRemoteRequestsButKeepsLocalPairingAndPreferences() {
    val stale = request("req-1", "E:\\project")
    val before = WorkbenchUiState(
      connectionState = BridgeConnectionState.Connected,
      paired = true,
      request = stale,
      requests = listOf(stale),
      userInput = "旧回复",
      selectedOptions = setOf("继续"),
      lightAppearance = false,
      notificationsEnabled = false,
    ).beginAction("req-1", "E:\\project", "action-1")
    val after = before.clearRemoteRequests(BridgeConnectionState.Offline, "网络连接中断")
    assertEquals(emptyList<MobileRequest>(), after.requests)
    assertEquals(null, after.request)
    assertEquals("", after.userInput)
    assertEquals(emptySet<String>(), after.selectedOptions)
    assertEquals(true, after.sending)
    assertEquals("action-1", after.pendingAction?.clientActionId)
    assertEquals("旧回复", after.drafts["req-1" to "E:\\project"]?.text)
    assertEquals(setOf("继续"), after.drafts["req-1" to "E:\\project"]?.selectedOptions)
    assertEquals(true, after.drafts["req-1" to "E:\\project"]?.deliveryUnknown)
    assertEquals(true, after.actionNotice?.contains("等待手机回复送达确认"))
    assertEquals(true, after.paired)
    assertEquals(false, after.lightAppearance)
    assertEquals(false, after.notificationsEnabled)
    assertEquals("网络连接中断", after.bridgeError)
  }

  @Test
  fun reconnectRestoresDraftOnlyForExactRequestAndNeverResends() {
    val first = request("req-1", "E:\\project")
    val other = request("req-2", "E:\\project")
    val disconnected = WorkbenchUiState(
      paired = true,
      connectionState = BridgeConnectionState.Connected,
      request = first,
      requests = listOf(first, other),
      userInput = "还没确认的回复",
      selectedOptions = setOf("继续"),
    ).beginAction("req-1", "E:\\project", "action-1")
      .clearRemoteRequests(BridgeConnectionState.Offline, "连接已断开")

    val differentWindow = disconnected.receiveRequest(other)
    assertEquals(null, differentWindow.request)
    assertEquals(listOf(other), differentWindow.requests)
    assertEquals("", differentWindow.userInput)
    assertEquals(true, differentWindow.sending)
    val outstanding = differentWindow.pendingAction
    assertEquals(true, outstanding?.matchesReceipt("req-1", "E:\\project", "action-1"))
    assertEquals(false, outstanding?.matchesReceipt("req-2", "E:\\project", "action-1"))
    assertEquals(false, outstanding?.matchesReceipt("req-1", "E:\\other", "action-1"))
    assertEquals(false, outstanding?.matchesReceipt("req-1", "E:\\project", "other-action"))

    val withDraftForOther = differentWindow.selectActiveRequest(other).copy(userInput = "另一个窗口的草稿")
    val suppressed = withDraftForOther.receiveRequest(first)
    assertEquals(withDraftForOther, suppressed)
    assertEquals("还没确认的回复", suppressed.drafts["req-1" to "E:\\project"]?.text)

    val delivered = suppressed.deliveredRequest("req-1", "E:\\project")
    assertEquals(other, delivered.request)
    assertEquals("另一个窗口的草稿", delivered.userInput)
    assertEquals(null, delivered.drafts["req-1" to "E:\\project"])
  }

  @Test
  fun unsentDraftSurvivesReconnectWithoutUnknownDeliveryWarning() {
    val active = request("req-1", "E:\\project")
    val offline = WorkbenchUiState(
      request = active,
      requests = listOf(active),
      userInput = "尚未发送",
    ).clearRemoteRequests(BridgeConnectionState.Offline)
    val back = offline.receiveRequest(active)
    assertEquals("尚未发送", back.userInput)
    assertEquals(null, back.actionNotice)
    assertEquals(false, back.sending)
  }

  @Test
  fun uncertainReceiptKeepsDraftAndWarningWhenSwitchingWindows() {
    val first = request("req-1", "E:\\project")
    val second = request("req-2", "E:\\project")
    val pending = WorkbenchUiState(
      request = first,
      requests = listOf(first, second),
      userInput = "请继续",
      sending = true,
    )
    val failed = pending.actionFailed("req-1", "E:\\project", ActionReceipt(false, "送达状态未知；请核对原窗口", true))
    assertEquals("请继续", failed.userInput)
    assertEquals(false, failed.sending)
    val switched = failed.selectActiveRequest(second)
    assertEquals("", switched.userInput)
    val restored = switched.selectActiveRequest(first)
    assertEquals("请继续", restored.userInput)
    assertEquals(true, restored.actionNotice?.contains("送达未知"))
  }

  @Test
  fun pendingReceiptKeepsOriginalActionIdAndPayloadForRetry() {
    val first = request("req-1", "E:\\project")
    val pending = WorkbenchUiState(request = first, requests = listOf(first), userInput = "请继续")
      .beginAction("req-1", "E:\\project", "action-1", "submit", "original-wire-payload")
    val unknown = pending.actionFailed("req-1", "E:\\project", ActionReceipt(false, "待确认", true))
    val unconfirmed = unknown.drafts["req-1" to "E:\\project"]?.unconfirmedAction
    assertEquals("action-1", unconfirmed?.clientActionId)
    assertEquals("original-wire-payload", unconfirmed?.payload)
    assertEquals(true, unknown.drafts["req-1" to "E:\\project"]?.deliveryUnknown)
  }

  @Test
  fun changedOptionsPruneRestoredAndCurrentlySelectedChoices() {
    val original = request("req-1", "E:\\project", listOf("继续", "停止"))
    val changed = request("req-1", "E:\\project", listOf("停止"))
    val other = request("req-2", "E:\\project", listOf("继续"))
    val before = WorkbenchUiState(
      request = original,
      requests = listOf(original, other),
      userInput = "保留这段文字",
      selectedOptions = setOf("继续"),
    )

    val restored = before.clearRemoteRequests(BridgeConnectionState.Offline).receiveRequest(changed)
    assertEquals("保留这段文字", restored.userInput)
    assertEquals(emptySet<String>(), restored.selectedOptions)
    assertEquals(emptySet<String>(), restored.drafts["req-1" to "E:\\project"]?.selectedOptions)

    val liveUpdate = before.receiveRequest(changed)
    assertEquals("保留这段文字", liveUpdate.userInput)
    assertEquals(emptySet<String>(), liveUpdate.selectedOptions)

    val switched = before.selectActiveRequest(other).receiveRequest(changed).selectActiveRequest(changed)
    assertEquals("保留这段文字", switched.userInput)
    assertEquals(emptySet<String>(), switched.selectedOptions)
    assertEquals(other, switched.requests.firstOrNull { it.requestId == "req-2" })
  }

  @Test
  fun closeBeforeDeliveredAckRetainsDraftUntilMatchingActionCompletes() {
    val first = request("req-1", "E:\\project")
    val other = request("req-2", "E:\\project")
    val pending = WorkbenchUiState(
      request = first,
      requests = listOf(first, other),
      userInput = "正在发送的内容",
      selectedOptions = setOf("继续"),
    ).beginAction("req-1", "E:\\project", "action-1")
    assertEquals(WorkbenchRoute.Conversation, pending.route)
    assertEquals(null, pending.request)
    assertEquals(listOf(other), pending.requests)
    assertEquals("", pending.userInput)

    val closed = pending.closeRequest("req-1", "E:\\project")
    assertEquals(WorkbenchRoute.Conversation, closed.route)
    assertEquals("正在发送的内容", closed.drafts["req-1" to "E:\\project"]?.text)
    assertEquals(true, closed.pendingAction?.closeObserved)
    assertEquals(true, closed.sending)
    assertEquals(null, closed.request)
    assertEquals("", closed.userInput)
    assertEquals(closed, closed.receiveRequest(first))

    val delivered = closed.deliveredRequest("req-1", "E:\\project")
    assertEquals(null, delivered.drafts["req-1" to "E:\\project"])
    assertEquals(null, delivered.pendingAction)
    assertEquals(false, delivered.sending)
    assertEquals(null, delivered.request)
    assertEquals(WorkbenchRoute.Conversation, delivered.route)
  }

  @Test
  fun closeBeforeUnknownAckRestoresConversationAndOriginalDraft() {
    val first = request("req-1", "E:\\project")
    val other = request("req-2", "E:\\project")
    val pending = WorkbenchUiState(
      request = first,
      requests = listOf(first, other),
      userInput = "请继续",
      selectedOptions = setOf("继续"),
    ).beginAction("req-1", "E:\\project", "action-1")
    val closed = pending.closeRequest("req-1", "E:\\project")
    val otherOpened = closed.selectActiveRequest(other).copy(userInput = "另一条草稿")
    val uncertain = otherOpened.actionFailed(
      "req-1",
      "E:\\project",
      ActionReceipt(false, "等待确认超时", true),
    )
    assertEquals("请继续", uncertain.drafts["req-1" to "E:\\project"]?.text)
    assertEquals(null, uncertain.pendingAction)
    assertEquals(false, uncertain.sending)
    assertEquals(true, uncertain.actionNotice?.contains("核对原窗口"))
    assertEquals(listOf(first, other), uncertain.requests)
    assertEquals(first, uncertain.request)
    assertEquals("请继续", uncertain.userInput)
    assertEquals(setOf("继续"), uncertain.selectedOptions)
    assertEquals(WorkbenchRoute.Conversation, uncertain.route)
    assertEquals("另一条草稿", uncertain.selectActiveRequest(other).userInput)

    val reconnected = uncertain.clearRemoteRequests(BridgeConnectionState.Offline)
    assertEquals(null, reconnected.pendingAction)
    assertEquals("请继续", reconnected.drafts["req-1" to "E:\\project"]?.text)
    assertEquals(false, reconnected.drafts["req-2" to "E:\\project"]?.deliveryUnknown)
  }

  @Test
  fun deliveredAckBeforeCloseAlsoClearsOnlyItsOwnDraft() {
    val first = request("req-1", "E:\\project")
    val other = request("req-2", "E:\\project")
    val before = WorkbenchUiState(
      request = first,
      requests = listOf(first, other),
      userInput = "已收到",
    ).beginAction("req-1", "E:\\project", "action-1")
    val delivered = before.deliveredRequest("req-1", "E:\\project")
    val closed = delivered.closeRequest("req-1", "E:\\project")
    assertEquals(listOf(other), closed.requests)
    assertEquals(null, closed.request)
    assertEquals(null, closed.drafts["req-1" to "E:\\project"])
  }
}
