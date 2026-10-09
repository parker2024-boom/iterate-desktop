# 中心与源适配器协议 v1

## 兼容依据与范围

只读依据为 `iterate-android/app/src/main/java/com/kexin94yyds/iterate/mobile/BridgeClient.kt`、`BridgeJsonParser.kt`，以及桌面 `src/rust/bridge/ws.rs`、`cross_device.rs`、`codex_questions.rs` 的字段和状态处理。没有修改 Android。

实际安装手机基线由主代理只读取证：USB 已授权 MNA_AL00，包 `com.kexin94yyds.iterate`，versionName 0.1.0/versionCode 1，lastUpdateTime 2026-10-02 19:54:23。当前源码契约不是该安装包实际运行结果；最终必须另行验收该安装版。

## 手机入口

| 入口 | 行为 |
| --- | --- |
| `GET /ws` | 原 Authorization、X-Iterate-Device-Id、X-Iterate-Client-Kind 鉴权；client_hello、request_sync、request_timeline_sync、mcp_action、cross_device_mirror_action |
| `POST /api/android/pairing/claim` | client_kind/android、pairing_token/pt_、device_id、可选 device_name；返回 ok/device_id/device_token/dt_/scopes/pairing_session_id |
| `GET /api/android/pairing/status` | 验证持久凭据与撤销状态；iOS 始终拒绝 |
| `GET /image?id=img_<32hex>` | 只读持久源/原资产/请求版本绑定，禁止路径/URL代理；8 MiB 上限 |
| `POST /api/open-codex-chat` | window.show 权限；仅定向 Windows，等源 RPC 回应，超时不返回成功 |
| `GET /`、`/index.html`、`/mobile` | session.read 权限；内容由授权 Windows 源提供，离线 503，确认未知 504 |

上述 Windows RPC 需要 TODO 2 的源适配器才能服务真实内容。主页面入口不是新建云端桌面管理 UI，也没有开放 Mac 文件/管理 API。APK 源码没有调用的桌面配置、APNS、隧道恢复、文件管理接口不自动转发；若后续真实资产证明是必要兼容接口，需逐项明确源权限后补齐，不能通配 HTTP 代理。

手机消息保持旧格式。`mcp_state`、`native_history_sessions`、`timeline_sync_snapshot/delta`、`cross_device_mirror_state` 来自已授权的对应源。无目标 request_sync 拆成 Windows 模板/native 状态 RPC 与中心已发布 Mac 镜像快照。时间线和动作按持久映射找唯一源，未知目标直接拒绝，绝不跨源广播兜底。

Mac 镜像必须为 `cross-mirror:<64位小写hex>`，source=`cross_device_mirror`，空 project_path；request.id 与 mirror_id/request_id 一致。只允许文字/选项镜像，不允许 Mac 图片资产上传或 Mac 全历史 RPC。

响应状态：

| 类型 | 未确认 | 确定拒绝 | 源持久确认 |
| --- | --- | --- | --- |
| Mac submit | pending | rejected | accepted / mac_accepted |
| Mac close_mirror | pending | rejected | accepted / mirror_closed |
| Windows普通 | pending、delivered=false | rejected | delivered、delivered=true |
| Windows native | pending、final_result=false | rejected、final_result=true | delivered、final_result=true、source=codex_native、result_kind=saved或cli_received |

旧 Android native_query 使用原 payload 加 native_query=true，只查询原动作，不创建新动作。内部 queued/confirmation_unknown/source_accepted/response_handoff 不直接作为手机 status。source_accepted 不代表 AI 已继续；response_handoff 仅代表源端自己的响应交接账本状态。

## 源出站连接

每台源独立 token，凭据以 SHA256 验证材料存储。每个请求带：

```text
Authorization: Bearer <该源专有token>
X-Iterate-Device-Id: <已登记源ID>
X-Iterate-Client-Kind: windows 或 macos
X-Iterate-Source-Session: <本次源适配器进程生命周期内固定的随机ID>
```

当前采用主动 HTTP 拉取而非源端入站监听；无须公开源机器端口。建议每 1 秒调用 `/source/next`，空闲可每 10 秒心跳。45 秒无本进程心跳视为离线；hub 重启后所有源需重新连接，不能用旧持久心跳冒充在线。Windows 离线关闭其活动快照，模板、native历史、页面与图片不可用；获准 Mac 已发布镜像仍可恢复显示，动作继续 pending，直到源账本确认。

源会话头是源 API 必填字段，不影响旧 Android。适配器每次进程启动生成新 ID；同进程后续轮询保留该 ID。会话 ID 改变、心跳过期后恢复或 hub 重启会推进源连接代次，使此前屏障失效。

桌面配对管理是例外：`POST /source/pairing`、`/source/pairing-session`、`/source/pairing-applied` 使用 Windows 源凭据及 `pairing.issue` scope，不更新在线会话或屏障代次。桌面仍必须通过原本机 renderer 身份校验才能发行 QR；admin 凭据不进入前端。发行前保留本机授权快照及 session 意图，后续只接受该意图对应且未扩大授权的 Android 配对记录。本机授权在发行后改变时拒绝覆盖。

QR 保持旧 APK 的 version=2、public_tunnel 和固定 8443 HTTPS/WSS；一次使用，120 秒有效。同一活动 Android 新配对保留既有 scopes/roots/Mac 授权并使旧 token 失效。用户主动扫描新 QR 可重新授权已撤销 Android，仅恢复默认 Windows scopes，清空旧 roots/Mac 授权；凭据迁移自身不解除撤销，历史 iOS 不恢复。云 claim 持久化、Windows 原授权文件持久化、带匹配身份的真实 WS hello 均完成后，session 才返回 connected。此协议测试不等于已安装 APK 或桌面 UI 验收。

| 接口 | 数据/用途 |
| --- | --- |
| `POST /source/heartbeat` | 只更新在线状态 |
| `POST /source/publish` | Publication：source_request_id、version>0、registration_key、route_id、kind(mcp/native/mirror)、project_path、epoch、published=true、wire |
| `POST /source/close` | source_request_id、version、registration_key；标记已发布请求关闭 |
| `GET /source/next` | 当前 epoch/frozen/drain（drain 为明确布尔值）、freeze_generation、source_generation、新 jobs、未知 reconcile、持久 cursor |
| `POST /source/authorize` | job_id；在源最终写账本之前再次核对手机权限/撤销、请求活动版本及 epoch |
| `POST /source/confirm` | 源动作账本确认，详见下节 |
| `POST /source/respond` | job_id、epoch、response；用于 sync/open_codex/page 的原源执行结果 |
| `POST /source/image` | epoch、original_asset_id(img_32hex)、source_request_id、version、mime、data_base64；返回全局 id/url |
| `POST /source/barrier` | epoch、freeze_generation、source_generation、legacy_connections_closed=true、all_submit_paths_paused=true；必须真实实现所有入口屏障后才能报告 |

route_id 一经绑定永不指向另一源或另一版本。旧手机不发版本号，因此适配器必须为新版本分配新的兼容 wire route，不能重用旧 ID。原 source_request_id、registration_key 保持在独立映射中，不能被云 wire alias 替代原账本键。publication 是不可变的已发布快照；重复完全相同的 publication 幂等。offline deferred 内容不得传 published=true；hub 不扫描源目录，也不会自动发布这些窗口。

图片先按源原图片 ID + 原请求 + 版本上传得到全局 ID，再只替换展示 wire 的 `/image?id=` 引用。源文件路径从不上传。重复相同图片返回原全局 ID，不同内容拒绝；数据持久化后重启不改变 ID。Mac 镜像不走该入口。

## 动作幂等与源账本

中心按 `(手机device_id, client_action_id)` 保存原 wire；同内容返回原状态，不同内容拒绝。job.payload 包含：

```json
{
  "action_id": "中心生成的ID",
  "source_request_id": "源原请求ID",
  "version": 1,
  "registration_key": "源原注册key",
  "wire": {"message_type":"旧手机原消息","payload":{}},
  "principal": {"device_id":"手机ID","client_kind":"android","scopes":[],"file_browser_roots":[],"windows_source":"win","mirror_sources":["mac"]}
}
```

原 wire 不添加 metadata。源适配器使用原有 response 构造方法，保留包含旧 metadata 的**原完整 response**；不能为了记录云 action_id 往 response 新增字段，使旧账本内容比较改变。云信息只放信封或独立映射。

`/source/next` 在交付前事务标记 dispatched。响应丢失、进程重启、断线后，已交付但未确认项只进入 `reconcile`，且 `query_only=true`；**不得重放执行**。源先查询其原账本，按原获胜记录确认。只有源持久证明未执行，才发送 `state=not_executed,durable=true,definitely_not_executed=true`，由中心重新放回待派发。15 秒手机超时不会触发盲重发或成功推断。

确认结构：

```json
{
  "action_id": "云动作ID",
  "source_request_id": "源原请求ID",
  "version": 1,
  "registration_key": "原注册key",
  "epoch": 1,
  "state": "source_accepted",
  "durable": true,
  "winner_record": "源端原持久获胜记录ID",
  "original_response": {"保持源原完整response":true},
  "receipt": {"message_type":"旧手机回执类型","payload":{}}
}
```

state 可为 unknown、not_executed、source_accepted、rejected、response_handoff。接受必须有 durable、winner_record、original_response 和满足旧手机规则的 receipt；中心拒绝错源/错注册key/错版本/错epoch/相冲突获胜记录。request→winner_action→源原 winner_record 与完整 response 持久绑定。拒绝也须源明确持久裁决，网络超时不能当拒绝。

source_accepted 后关闭对应活动快照，并继续保存原回执供同动作重试。response_handoff 只能在原接受记录上转移，不能改变获胜记录或把它解释为 AI 后续输出。实际 AI 输出仍由后续真实 CLI 验收证明。

最终执行去重责任仍在源。TODO 2 必须让 Windows/Mac 的云、本地、旧连接、后台恢复共用同一个原请求锁和获胜账本；本中心的组件测试不能证明尚未修改的源端已经统一。

## Windows RPC

sync 任务 response=`{"events":[旧Bridge消息...]}`。mcp_state 可提供 customPrompts 同步信息，但 request 对象必须通过 /source/publish 单独登记；不能借 sync 发布跨源请求。native_history_sessions 中的 native-cli 路由持久绑定 Windows，用于后续时间线定位。timeline 响应必须匹配已绑定源/路由/项目。

WS 发起的 sync/timeline RPC 同时持久绑定手机身份和中心生成的当前 WS 会话 ID，只投递到该会话。只有模板 sync 使用 sync_reason 关联；旧手机不带 sync_reason 的时间线请求仍按任务、路由和会话投递。其他手机、同手机另一条 WS 及重连后的新会话不能收到该 RPC 的历史响应；重连需要重新请求时间线。

open_codex 的 response 是源实际结果 JSON，只有源返回才传给 HTTP 客户端。page 的 response 为 `{"status":200,"body":"源返回的HTML"}`。原 Windows 路径和 file_browser_roots 只传给原源判定，不在中心改写或扩大。page/open_codex 未确认时，HTTP 超时返回不可确认而非成功；相关 dispatched job 仍保留用于对账，不删除或自动再次执行。

## 管理、冻结和升级

管理员 token 与设备凭据完全分离：

- `POST /admin/provision`：`{kind:source|phone,credential:{...}}`。phone 保留 client_kind、scopes、revoked_at、file_browser_roots，加明确 windows_source 与已授权 mirror_sources。原身份只能完全相同地重试导入，不能隐式扩权覆盖。
- `POST /admin/pairing`：传新的 Android grant，返回新 pt_ 二维码内容，120 秒有效且只能消费一次；数据库只存哈希。禁止导入旧未消费二维码。
- `POST /admin/revoke`：kind/device_id；持久撤销。手机每条 WS 消息及每 500 ms 快照检查重新鉴权；撤销会关闭已有连接，未派发任务不再执行。源最终提交前另须 /source/authorize。
- `GET /admin/status`：容量、epoch、queued/unknown数量及动作状态/获胜记录ID，不含 token 或回复正文全集。
- `POST /admin/control`：`{"frozen":true,"drain":true}` 拒绝新入队但继续排空既有已知任务；`{"frozen":true,"drain":false}` 停止派发；未知项只查询源账本。冻结或 drain 状态实际改变时递增持久 freeze_generation。`{"advance_epoch":true}` 仅在完全冻结、无未决任务且每个启用源仍在线，并已报告当前 epoch、freeze_generation、source_generation 及 hub 进程对应的全入口屏障后推进。

屏障只在 frozen=true、drain=false 时接受。源读取 `/source/next` 的两个 generation 后真实建立屏障，并原样回传；不能推测代次或复用先前证明。解冻再冻结，即使 epoch 未变也必须重新取得全部源屏障；进入排空再回到完全冻结同样如此。

屏障由源适配器真实关闭旧产品转发/长连接、暂停本地与后台恢复提交后报告。仅断开新 websocket 不够。5311 主会话不被中心停止。数据库 schema 版本为1；旧版程序遇到更高版本拒绝打开，不自动降级、删表或重建账本。回退保留云和各源映射及未知记录，禁止清库后改 action_id 规避未知结果。

## 限额与状态恢复

JSON/WS 最大512 KiB；图片二进制最大8 MiB，base64请求最大12 MiB；固定loopback地址、防任意URL/文件代理、SQL参数化。账号、快照、回复、资产全部消耗持久容量预算。center不采集仓库、全配置或未发布窗口，日志只输出启动地址和通用错误。

源有单调job序号、完成游标及逐job状态；游标是完成序号水位，不表示前面所有未知job都已完成，reconcile逐项账本才是权威。手机没有逐事件 ACK：重连恢复当前活动/关闭快照和原 action 状态，不宣称逐事件可靠消费。临时 WS 的内存去重只保留上一轮当前快照集合；Windows 离线关闭后恢复时，同一条 WS 会重新收到相同的活动快照，不累积整条连接的历史哈希，也不参与最终动作去重。
