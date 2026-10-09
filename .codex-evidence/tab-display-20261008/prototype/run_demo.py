"""Temporary interactive launcher. All messages here are simulated."""
import argparse
import json
from pathlib import Path
from PySide6.QtCore import QTimer, Qt
from PySide6.QtWidgets import QApplication, QPushButton
from window import ConversationWindow
from demo_data import CONVERSATIONS


def make_demo():
    window = ConversationWindow()
    for item in CONVERSATIONS:
        window.add_conversation(item["id"], item["title"], item["project"], item["message"], item["options"])
    log = []
    def sent(key, text, options):
        log.append({"key": key, "text": text, "options": options})
        window.panes[key].mark_sent(text, options)
        window.refresh_labels()
        window.feedback.setText("模拟发送完成 · 标签保留，等待该会话后续消息")
    window.reply_requested.connect(sent)
    counter = [0]
    def incoming():
        counter[0] += 1
        window.add_conversation(f"demo-new-{counter[0]}", f"新会话 {counter[0]}", "示例项目",
                                "这是一条模拟后台消息。当前编辑页、草稿和光标应保持原样。")
    add = QPushButton("3 秒后模拟新会话")
    add.setObjectName("demo-add")
    add.clicked.connect(lambda: QTimer.singleShot(3000, incoming))
    follow = QPushButton("模拟当前会话的后续消息")
    follow.clicked.connect(lambda: window.add_conversation(window.current_key(), "", "", "后续结果已到达。可以继续回复。"))
    window.demo_controls.addWidget(add)
    window.demo_controls.addWidget(follow)
    window.demo_controls.addStretch()
    return window, log, add


def self_test(app, window, log, add, evidence):
    from PySide6.QtTest import QTest
    from PySide6.QtCore import QObject, QEvent
    import ctypes
    import time
    report = {"result": "Fail", "steps": [], "prototype_only": True}
    def foreground():
        user32 = ctypes.windll.user32
        user32.GetForegroundWindow.restype = ctypes.c_void_p
        hwnd = user32.GetForegroundWindow()
        title = ctypes.create_unicode_buffer(512)
        user32.GetWindowTextW(ctypes.c_void_p(hwnd), title, len(title))
        return title.value
    class Events(QObject):
        def eventFilter(self, obj, event):
            if event.type() in (QEvent.Type.WindowActivate, QEvent.Type.WindowDeactivate) and obj.isWindow():
                report.setdefault("activation", []).append({"time": time.monotonic(), "event": str(event.type()), "object": type(obj).__name__, "foreground": foreground()})
            return False
    events = Events()
    app.installEventFilter(events)
    def click(widget):
        QTest.mouseClick(widget, Qt.MouseButton.LeftButton)
        QTest.qWait(100)
    def tab(index):
        QTest.mouseClick(window.tabs.tabBar(), Qt.MouseButton.LeftButton, pos=window.tabs.tabBar().tabRect(index).center())
        QTest.qWait(100)
    try:
        window.show()
        window.raise_()
        window.activateWindow()
        QTest.qWait(700)
        first, second = window.panes["demo-layout"], window.panes["demo-f"]
        click(first.editor)
        QTest.keyClicks(first.editor, "draft-A")
        tab(1)
        click(second.editor)
        QTest.keyClicks(second.editor, "draft-B")
        tab(0)
        assert first.editor.toPlainText() == "draft-A" and second.editor.toPlainText() == "draft-B"
        report["steps"].append("tab switching preserves independent drafts")
        click(add)
        click(first.editor)
        QTest.keyClick(first.editor, Qt.Key.Key_Left)
        cursor = first.editor.textCursor().position()
        assert window.focusWidget() is first.editor
        report["focus_before"] = {"editor": app.focusWidget() is first.editor, "widget": str(app.focusWidget()), "active": str(app.activeWindow()), "cursor": cursor}
        QTest.qWait(3200)
        report["focus_after"] = {"editor": app.focusWidget() is first.editor, "widget": str(app.focusWidget()), "active": str(app.activeWindow()), "cursor": first.editor.textCursor().position()}
        assert window.current_key() == first.key and window.tabs.count() == 4
        assert window.focusWidget() is first.editor and first.editor.textCursor().position() == cursor
        if app.activeWindow() is window:
            assert app.focusWidget() is first.editor
        assert window.panes["demo-new-1"].unread
        report["steps"].append("background tab retains active editor and caret, shows unread badge")
        click(window.mode)
        QTest.keyClick(window.mode, Qt.Key.Key_Home)
        QTest.keyClick(window.mode, Qt.Key.Key_Enter)
        QTest.qWait(200)
        assert len(window.split_windows) == 4
        assert all(w.isVisible() for w in window.split_windows.values())
        click(window.mode)
        QTest.keyClick(window.mode, Qt.Key.Key_End)
        QTest.keyClick(window.mode, Qt.Key.Key_Enter)
        QTest.qWait(200)
        assert window.tabs.count() == 4 and not window.split_windows
        assert first.editor.toPlainText() == "draft-A" and second.editor.toPlainText() == "draft-B"
        report["steps"].append("windows/tabs display switch preserves all drafts")
        tab(1)
        click(second.send_button)
        assert log[-1]["key"] == second.key and log[-1]["text"] == "draft-B"
        assert first.editor.toPlainText() == "draft-A" and not second.waiting
        report["steps"].append("send affects only selected conversation and keeps its tab")
        tab(0)
        click(first.end_button)
        assert first.key not in window.panes
        click(window.restore_button)
        assert window.panes[first.key].editor.toPlainText() == "draft-A"
        report["steps"].append("close and restore retain the original draft")
        window.grab().save(str(evidence / "prototype.png"))
        report["result"] = "Pass"
    except Exception:
        import traceback
        report["error"] = traceback.format_exc()
    finally:
        (evidence / "acceptance.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
        print(json.dumps(report, ensure_ascii=False))
        window.close()
    return 0 if report["result"] == "Pass" else 1


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--evidence", required=True)
    args = parser.parse_args()
    evidence = Path(args.evidence)
    evidence.mkdir(parents=True, exist_ok=True)
    app = QApplication([])
    app.setApplicationName("Iterate tab display prototype")
    window, log, add = make_demo()
    window.show()
    if args.self_test:
        return self_test(app, window, log, add, evidence)
    QTimer.singleShot(500, lambda: window.grab().save(str(evidence / "preview.png")))
    return app.exec()


if __name__ == "__main__":
    raise SystemExit(main())
