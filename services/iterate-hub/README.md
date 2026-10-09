# iterate-hub

独立 Rust/Axum/SQLite 后台中心。这个 crate 有自己的 manifest、lockfile 和 workspace，不引用桌面 crate，不依赖 Tauri、WebKit 或 Wry。本阶段只实现和测试中心，不连接真实 Windows/Mac，不更改 Android，不部署。

## 构建与运行

Windows 本机构建示例（从桌面仓根目录执行）：

```powershell
$root='G:\DevLocal\BuildTemp\iterate-desktop\aliyun-center\hub'
$env:CARGO_TARGET_DIR="$root\cargo-target"
$env:TEMP="$root\tmp"
$env:TMP=$env:TEMP
cargo metadata --manifest-path services/iterate-hub/Cargo.toml --locked --offline --format-version 1
cargo tree --manifest-path services/iterate-hub/Cargo.toml --locked --offline
cargo build --manifest-path services/iterate-hub/Cargo.toml --locked --offline
cargo test --manifest-path services/iterate-hub/Cargo.toml --locked --offline
```

运行必须显式给出数据库路径；进程仅允许监听 loopback。管理员验证材料从 `ITERATE_HUB_ADMIN_HASH=sha256:<64位小写hex>` 注入，原始管理员 token 不放命令行或数据库。各设备的独立 token 通过管理员接口登记 SHA256；源设备与管理员不得共用 token。

```text
iterate-hub --db <private-data-dir>/hub.sqlite --listen 127.0.0.1:8444
```

默认且最高数据预算为 1 GiB，可用 `--budget-bytes` 降低。SQLite 使用 FULL 同步、DELETE journal、唯一主键和事务，不在重启时删除正文、动作或映射。新条目采用保守的 80% 计费水位，预留动作确认空间和数据库页开销；达到限制明确返回 `capacity_exceeded`/HTTP 507。SQLite 主文件另设 `max_page_count` 上限。事务回滚日志、临时文件及备份需要额外磁盘余量，1 GiB 是数据预算，不代表所有备份及瞬时磁盘占用总计只有 1 GiB。

本服务不自动清理已有正文/去重账本。运维应监控 `/admin/status` 的容量、queued、unknown；达到预算时停止新任务，保留现有数据并明确协调，不能通过清空数据库解锁。

## 本阶段实测

`tests/component.rs` 测试 SQLite 重启、同动作重试/冲突、源绑定与胜者记录、旧回执字段、过期版本、IOS/撤销/权限、一次性配对、Windows 离线与 Mac 镜像、图片跨重启与会话隔离、容量拒收后完成已入队动作、冻结/排空/epoch、模板与 native 时间线定源、Windows 页面与打开 Codex 的 RPC 契约。

`tests/process.rs` 启动真实 `iterate-hub` 子进程，以临时凭据通过 HTTP 与 WS 操作；在源确认前结束这个测试子进程，重用同一数据库启动，检查只查询未知动作、源确认后才接受、旧 WS 连接在手机撤销后关闭。测试只终止自己创建的子进程。

独立 review 首轮发现的三个缺陷另有真实 HTTP/WS 回归：同 epoch 解冻/再冻结、排空切换与源会话改变均拒绝旧屏障；非空 sync_reason 的旧手机连接可接收不带该字段的时间线，且不投递到其他手机或 WS 会话；同一 WS 等待真实 45 秒心跳过期后收到关闭，心跳恢复后再次收到相同活动快照。15 项组件测试及 4 项进程测试通过，仅表示执行人自检，仍须独立复验。

这些是中心组件和独立进程证据。尚未连接真实源账本、真实安装 APK、真实 TLS 入口、生产数据或 USB 手机，不能据此宣称最终端到端兼容通过。

## 后续接线与部署边界

1. TODO 2 的 Windows/Mac 适配器按 [PROTOCOL.md](PROTOCOL.md) 出站连接；不得凭 `queued`、HTTP 200 或 hub 的持久化宣称源已接收。源端必须把云、本地、旧长连接、后台恢复提交纳入同一原账本及原请求锁。
2. 既有 `https://8.129.82.226:8443`/`wss://8.129.82.226:8443/ws` 对外地址保持不变，由已有 TLS 入口转到 loopback hub。部署前核对真实 IP 证书及设备信任方式，不可关闭证书或主机身份验证。此 crate 不提供 `accept_invalid_certs` 开关，不创建或替换证书。
3. 在 TLS 代理层限制 `/admin/*` 到运维通道；源路由使用独立设备凭据，手机路由使用原有 dt_ 凭据。拒绝把管理员 token 配给手机。代理日志不得记录 Authorization、请求正文或完整带凭据查询串。
4. 私有数据目录权限仅给服务账户（Unix 0700，数据库 0600；Windows 等价 ACL）。只从已授权源记录导入已配对设备的哈希、client_kind、scopes、revoked_at、file_browser_roots；不导入未消费旧二维码。不修改 roots 字符串，不把 Windows 路径翻译成 Mac 权限。
5. 云构建目录 `/var/tmp/iterate-build/aliyun-center-20261004` 最高 5 GiB；Mac 构建例外目录 `/Users/parker/Library/Caches/iterate-build/aliyun-center-20261004`。此阶段没有在这些远端路径执行构建或部署。
6. 切换前先冻结新入队并排空已知任务，未知结果只查源账本；所有旧长连接和本地提交/恢复入口须由适配器确认关闭/暂停。epoch 只在无未决任务且每个启用源确认屏障后推进。保留云和源数据库以及旧映射，回退不能创建新 action_id 重试未知旧动作。

Mytool 仅托管后台，桌面 GUI 手动启动；5311 当前主会话不在本服务的关闭范围。
