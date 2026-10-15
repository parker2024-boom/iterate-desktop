# iterate-desktop 项目规则

- 项目临时产物：`G:\DevLocal\BuildTemp\iterate-desktop\<工具或任务>`。
- 构建、缓存与清理遵循 [task-artifact-storage](C:/Users/liguo/.agents/skills/task-artifact-storage/SKILL.md)，按当前工具读取其构建参数参考。
- 构建涉及 Cargo、Vite 和 Tauri；入口见 [package.json](package.json)。
