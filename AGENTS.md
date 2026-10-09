# 项目协作规则

## 编译与打包文件位置

- 编译过程文件、中间产物、增量缓存、构建输出和打包暂存统一放在 `G:\DevLocal\BuildTemp\iterate-desktop\<工具或任务>`，保留源码、任务数据和验收证据的原位置。
- 启动编译前，通过工具原生输出参数、构建配置或本次命令的环境变量明确指定 G 盘目录；既有脚本和文档中的仓库内输出路径按本条改为 G 盘后再使用。
- Cargo 使用命令级 `CARGO_TARGET_DIR` 或 `--target-dir`；Vite 使用 `--outDir`；Tauri/安装包输出与暂存也放入同一 G 盘项目目录。
- 本次编译涉及的临时文件通过命令级 `TEMP` / `TMP` 指向上述 G 盘目录下的临时子目录。
- 每次实际构建后检查真实落盘路径，并在交付中写明构建、缓存与打包输出目录；此规则本身不等于构建配置已经完成迁移。
- Python 相关过程或测试命令启动前，将命令级 `PYTHONPYCACHEPREFIX` 设为 `G:\DevLocal\BuildTemp\iterate-desktop\python-cache`，使自动生成的 `.pyc` 缓存落在 G 盘。
