"""Reusable Qt presentation; transports are supplied through signals."""
from PySide6.QtCore import Qt, Signal, QTimer
from PySide6.QtGui import QFont, QTextCursor
from PySide6.QtWidgets import (
    QMainWindow, QWidget, QVBoxLayout, QHBoxLayout, QLabel, QPushButton,
    QComboBox, QTabWidget, QTextBrowser, QTextEdit, QFrame, QToolButton,
)

STYLE = """
QWidget { background:#18181c; color:#e6e6eb; font:10pt 'Microsoft YaHei UI'; }
QLabel#muted { color:#a1a1ae; }
QLabel#notice { background:#292b30; color:#d7c58d; padding:7px 12px; border-radius:6px; }
QLabel#title { font-size:17px; font-weight:600; }
QPushButton,QToolButton,QComboBox { background:#2c2c32; border:1px solid #41414a; border-radius:6px; padding:7px 12px; }
QPushButton:hover,QToolButton:hover { background:#393940; }
QPushButton:disabled { color:#686873; border-color:#303038; }
QPushButton#primary { background:#63e2b7; color:#122820; border-color:#63e2b7; font-weight:600; }
QPushButton:checked { background:#21483c; border-color:#63e2b7; color:#9bf1d1; }
QComboBox { min-width:115px; }
QComboBox QAbstractItemView { background:#242429; selection-background-color:#3c4b44; }
QTabWidget::pane { border:1px solid #37373f; border-radius:7px; }
QTabBar::tab { background:#242429; color:#aaaab6; border:1px solid #35353d; padding:10px 15px; min-width:110px; max-width:210px; }
QTabBar::tab:selected { background:#303039; color:#ffffff; border-bottom:2px solid #63e2b7; }
QTextBrowser { background:#202026; border:1px solid #34343d; border-radius:7px; padding:12px; }
QTextEdit { background:#202026; border:1px solid #454550; border-radius:7px; padding:10px; selection-background-color:#356b58; }
QTextEdit:focus { border-color:#63e2b7; }
QFrame#separator { background:#34343b; max-height:1px; }
"""


class ConversationPane(QWidget):
    submitted = Signal(str, str, list)
    end_requested = Signal(str)

    def __init__(self, key, title, project, message, options=()):
        super().__init__()
        self.key, self.title, self.project = key, title, project
        self.unread = False
        self.waiting = True
        self.option_buttons = []
        layout = QVBoxLayout(self)
        layout.setContentsMargins(18, 16, 18, 16)
        layout.setSpacing(12)
        title_row = QHBoxLayout()
        title_label = QLabel(title)
        title_label.setObjectName("title")
        title_row.addWidget(title_label)
        title_row.addStretch()
        self.state_label = QLabel("等待回复")
        self.state_label.setObjectName("muted")
        title_row.addWidget(self.state_label)
        layout.addLayout(title_row)
        project_label = QLabel(project)
        project_label.setObjectName("muted")
        layout.addWidget(project_label)
        self.history = QTextBrowser()
        self.history.setPlainText(message)
        layout.addWidget(self.history, 3)
        option_row = QHBoxLayout()
        for option in options:
            button = QPushButton(option)
            button.setCheckable(True)
            self.option_buttons.append(button)
            option_row.addWidget(button)
            button.toggled.connect(self.update_send)
        option_row.addStretch()
        layout.addLayout(option_row)
        self.editor = QTextEdit()
        self.editor.setAccessibleName(f"{title}的回复")
        self.editor.setPlaceholderText("输入回复；切换标签会保留本会话的草稿")
        self.editor.setMinimumHeight(100)
        layout.addWidget(self.editor, 2)
        footer = QHBoxLayout()
        self.draft_label = QLabel("草稿仅属于当前会话")
        self.draft_label.setObjectName("muted")
        footer.addWidget(self.draft_label)
        footer.addStretch()
        self.end_button = QPushButton("结束会话")
        self.end_button.clicked.connect(lambda: self.end_requested.emit(self.key))
        footer.addWidget(self.end_button)
        self.send_button = QPushButton("发送")
        self.send_button.setObjectName("primary")
        self.send_button.clicked.connect(self.send)
        footer.addWidget(self.send_button)
        layout.addLayout(footer)
        self.editor.textChanged.connect(self.update_send)
        self.update_send()

    def update_send(self):
        text = self.editor.toPlainText()
        self.send_button.setEnabled(self.waiting and bool(text.strip() or any(b.isChecked() for b in self.option_buttons)))
        self.draft_label.setText(f"本会话草稿 · {len(text)} 字" if text else "草稿仅属于当前会话")

    def send(self):
        text = self.editor.toPlainText().strip()
        options = [b.text() for b in self.option_buttons if b.isChecked()]
        if not self.waiting or not (text or options):
            return
        self.submitted.emit(self.key, text, options)

    def mark_sent(self, text, options):
        self.history.append("\n你：\n" + "\n".join([*options, text]).strip())
        self.history.append("\n已发送，等待后续消息。")
        self.editor.clear()
        for b in self.option_buttons:
            b.setChecked(False)
        self.waiting = False
        self.state_label.setText("处理中")
        self.update_send()

    def receive(self, message):
        self.history.append("\n新消息：\n" + message)
        self.waiting = True
        self.state_label.setText("等待回复")
        self.update_send()


class SplitWindow(QMainWindow):
    closed = Signal(str)

    def __init__(self, pane):
        super().__init__()
        self.key = pane.key
        self.migrating = False
        self.setWindowTitle(f"{pane.title} · Iterate 分窗原型")
        self.setStyleSheet(STYLE)
        self.resize(570, 650)
        self.setCentralWidget(pane)

    def closeEvent(self, event):
        if not self.migrating:
            self.closed.emit(self.key)
        event.accept()


class ConversationWindow(QMainWindow):
    reply_requested = Signal(str, str, list)
    conversation_closed = Signal(str)
    mode_changed = Signal(str)

    def __init__(self):
        super().__init__()
        self.setWindowTitle("Iterate · 标签页显示方式 · 交互原型")
        self.resize(990, 790)
        self.setMinimumSize(720, 550)
        self.setStyleSheet(STYLE)
        self.panes = {}
        self.split_windows = {}
        self.archived = []
        self._switching = False
        self._selected_key = None
        shell = QWidget()
        layout = QVBoxLayout(shell)
        layout.setContentsMargins(16, 12, 16, 12)
        layout.setSpacing(12)
        top = QHBoxLayout()
        brand = QLabel("∞  iterate")
        brand.setObjectName("title")
        top.addWidget(brand)
        top.addStretch()
        mode_label = QLabel("显示方式")
        mode_label.setObjectName("muted")
        top.addWidget(mode_label)
        self.mode = QComboBox()
        self.mode.setAccessibleName("显示方式")
        self.mode.addItems(["分窗", "标签页"])
        self.mode.setCurrentIndex(1)
        self.mode.currentIndexChanged.connect(self.switch_mode)
        top.addWidget(self.mode)
        self.restore_button = QPushButton("恢复关闭的会话")
        self.restore_button.setEnabled(False)
        self.restore_button.clicked.connect(self.restore_last)
        top.addWidget(self.restore_button)
        layout.addLayout(top)
        notice = QLabel("交互原型 · 未接入真实业务   |   现有分窗保留，新增标签页选项")
        notice.setObjectName("notice")
        layout.addWidget(notice)
        self.tabs = QTabWidget()
        self.tabs.setMovable(True)
        self.tabs.setTabsClosable(True)
        self.tabs.setDocumentMode(True)
        self.tabs.tabBar().setExpanding(False)
        self.tabs.tabBar().setElideMode(Qt.TextElideMode.ElideRight)
        self.tabs.tabCloseRequested.connect(lambda index: self.close_conversation(self.tabs.widget(index).key))
        self.tabs.currentChanged.connect(self.select_current)
        layout.addWidget(self.tabs, 1)
        self.split_notice = QLabel("当前使用分窗显示。切换到“标签页”可合并到一个窗口。")
        self.split_notice.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.split_notice.hide()
        layout.addWidget(self.split_notice, 1)
        self.feedback = QLabel("后台新消息只更新标签，不打断当前编辑")
        self.feedback.setObjectName("muted")
        layout.addWidget(self.feedback)
        self.demo_controls = QHBoxLayout()
        layout.addLayout(self.demo_controls)
        self.setCentralWidget(shell)

    def add_conversation(self, key, title, project, message, options=(), *, background=True):
        if key in self.panes:
            self.panes[key].receive(message)
            if key != self.current_key():
                self.panes[key].unread = True
            self.refresh_labels()
            return self.panes[key]
        pane = ConversationPane(key, title, project, message, options)
        pane.submitted.connect(self.reply_requested)
        pane.end_requested.connect(self.close_conversation)
        pane.editor.textChanged.connect(self.refresh_labels)
        self.panes[key] = pane
        if self.mode.currentIndex() == 1:
            previous = self.tabs.currentWidget()
            pane.unread = bool(background and previous)
            self.tabs.addTab(pane, title)
            if previous and background:
                self.tabs.setCurrentWidget(previous)
            else:
                self.tabs.setCurrentWidget(pane)
        else:
            self.show_split(pane)
        self.feedback.setText(f"{title} 收到新消息 · 当前共有 {len(self.panes)} 个会话")
        self.refresh_labels()
        return pane

    def current_key(self):
        current = self.tabs.currentWidget()
        return current.key if current else self._selected_key

    def select_current(self, index):
        if self._switching or index < 0:
            return
        pane = self.tabs.widget(index)
        self._selected_key = pane.key
        pane.unread = False
        self.refresh_labels()

    def refresh_labels(self):
        for i in range(self.tabs.count()):
            pane = self.tabs.widget(i)
            prefix = "● " if pane.unread else ""
            suffix = " · 草稿" if pane.editor.toPlainText() else (" · 处理中" if not pane.waiting else "")
            self.tabs.setTabText(i, prefix + pane.title + suffix)
            self.tabs.setTabToolTip(i, f"{pane.project}\n{pane.title}")

    def show_split(self, pane):
        window = SplitWindow(pane)
        window.closed.connect(self.close_conversation)
        self.split_windows[pane.key] = window
        screen = self.screen().availableGeometry()
        offset = 32 * (len(self.split_windows) - 1)
        window.move(screen.x() + 100 + offset, screen.y() + 80 + offset)
        window.show()

    def switch_mode(self, index):
        self._switching = True
        if index == 0:
            self._selected_key = self.current_key()
            ordered = [self.tabs.widget(i) for i in range(self.tabs.count())]
            while self.tabs.count():
                self.tabs.removeTab(0)
            self.tabs.hide()
            self.split_notice.show()
            for pane in ordered:
                self.show_split(pane)
        else:
            self.split_notice.hide()
            self.tabs.show()
            for key, window in list(self.split_windows.items()):
                pane = window.takeCentralWidget()
                window.migrating = True
                window.close()
                window.deleteLater()
                if pane:
                    self.tabs.addTab(pane, pane.title)
            self.split_windows.clear()
            if self._selected_key in self.panes:
                self.tabs.setCurrentWidget(self.panes[self._selected_key])
        self._switching = False
        self.select_current(self.tabs.currentIndex())
        self.refresh_labels()
        self.feedback.setText("已切换显示方式，各会话草稿保留")
        self.mode_changed.emit("tabs" if index else "windows")

    def close_conversation(self, key):
        pane = self.panes.pop(key, None)
        if pane is None:
            return
        window = self.split_windows.pop(key, None)
        if window:
            window.takeCentralWidget()
            window.migrating = True
            window.close()
            window.deleteLater()
        tab = self.tabs.indexOf(pane)
        if tab >= 0:
            self.tabs.removeTab(tab)
        pane.setParent(None)
        pane.hide()
        self.archived.append(pane)
        self.restore_button.setEnabled(True)
        self.feedback.setText(f"已关闭 {pane.title}，可恢复会话及草稿")
        self.conversation_closed.emit(key)

    def restore_last(self):
        if not self.archived:
            return
        pane = self.archived.pop()
        self.panes[pane.key] = pane
        if self.mode.currentIndex() == 1:
            self.tabs.addTab(pane, pane.title)
            self.tabs.setCurrentWidget(pane)
        else:
            self.show_split(pane)
        self.restore_button.setEnabled(bool(self.archived))
        self.refresh_labels()
        self.feedback.setText(f"已恢复 {pane.title} 和原草稿")

    def closeEvent(self, event):
        for window in self.split_windows.values():
            window.migrating = True
            window.close()
        event.accept()
