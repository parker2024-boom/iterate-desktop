use crate::{protocol::*, store::Store};
use axum::{
    body::Bytes,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        DefaultBodyLimit, State,
    },
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{any, get},
    Json, Router,
};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub struct Hub {
    pub store: Arc<Mutex<Store>>,
    admin_hash: String,
}
impl Hub {
    pub fn new(store: Store, admin_hash: String) -> anyhow::Result<Self> {
        anyhow::ensure!(hash_valid(&admin_hash), "admin_hash_required");
        Ok(Self {
            store: Arc::new(Mutex::new(store)),
            admin_hash,
        })
    }
    fn admin(&self, h: &HeaderMap) -> anyhow::Result<()> {
        let token = bearer(h);
        anyhow::ensure!(
            token.len() >= 20
                && bool::from(
                    hash(token.as_bytes())
                        .as_bytes()
                        .ct_eq(self.admin_hash.as_bytes())
                ),
            "admin_required"
        );
        Ok(())
    }
}
fn bearer(h: &HeaderMap) -> &str {
    h.get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("")
}
fn header<'a>(h: &'a HeaderMap, name: &str) -> &'a str {
    h.get(name).and_then(|v| v.to_str().ok()).unwrap_or("")
}
fn auth(db: &Store, h: &HeaderMap, kind: &str) -> anyhow::Result<Credential> {
    db.authenticate(
        kind,
        header(h, "x-iterate-device-id"),
        header(h, "x-iterate-client-kind"),
        bearer(h),
    )
}
fn failure(e: anyhow::Error) -> Response {
    let detail = e.to_string();
    let status = if detail.contains("credential") || detail == "admin_required" {
        StatusCode::UNAUTHORIZED
    } else if detail.contains("capacity") || detail.contains("database or disk is full") {
        StatusCode::INSUFFICIENT_STORAGE
    } else if detail.contains("scope")
        || detail.contains("permission")
        || detail.contains("forbidden")
    {
        StatusCode::FORBIDDEN
    } else if detail == "windows_offline" {
        StatusCode::SERVICE_UNAVAILABLE
    } else if detail.contains("conflict") || detail.contains("stale") || detail.contains("frozen") {
        StatusCode::CONFLICT
    } else {
        StatusCode::BAD_REQUEST
    };
    // Do not expose SQLite values, raw SQL, tokens or request contents.
    let code = if detail.len() <= 96 && detail.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
    {
        detail
    } else {
        "request_failed".into()
    };
    (status, Json(json!({"ok":false,"error":code}))).into_response()
}
pub fn router(hub: Hub) -> Router {
    Router::new()
        .route(
            "/health",
            get(|| async { Json(json!({"status":"ok","service":"iterate-hub","protocol":1})) }),
        )
        .route("/ws", get(upgrade))
        .route("/api/android/pairing/claim", any(api))
        .route("/api/android/pairing/status", any(api))
        .route("/api/open-codex-chat", any(open_codex))
        .route("/", get(main_page))
        .route("/index.html", get(main_page))
        .route("/mobile", get(main_page))
        .route("/image", get(image))
        .route("/source/:operation", any(api))
        .route("/admin/:operation", any(api))
        .layer(DefaultBodyLimit::max(12 * 1024 * 1024))
        .with_state(hub)
}
async fn api(
    State(hub): State<Hub>,
    method: Method,
    uri: Uri,
    h: HeaderMap,
    body: Bytes,
) -> Response {
    let result = (|| -> anyhow::Result<Value> {
        let mut db = hub
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store_unavailable"))?;
        let path = uri.path();
        anyhow::ensure!(
            body.len()
                <= if path == "/source/image" {
                    12 * 1024 * 1024
                } else {
                    MAX_MESSAGE
                },
            "message_too_large"
        );
        let v: Value = if body.is_empty() {
            json!({})
        } else {
            serde_json::from_slice(&body)?
        };
        if path.starts_with("/admin/") {
            hub.admin(&h)?;
            if path == "/admin/status" && method == Method::GET {
                return db.status();
            }
            anyhow::ensure!(method == Method::POST, "method_not_allowed");
            match path {
                "/admin/provision" => {
                    anyhow::ensure!(
                        field(&v["credential"], "token_hash") != hub.admin_hash,
                        "admin_device_credentials_must_differ"
                    );
                    db.provision(
                        field(&v, "kind"),
                        serde_json::from_value(v["credential"].clone())?,
                    )?;
                }
                "/admin/revoke" => {
                    db.revoke(field(&v, "kind"), field(&v, "device_id"))?;
                }
                "/admin/pairing" => return db.pairing(serde_json::from_value(v)?),
                "/admin/control" => return db.control(v),
                _ => anyhow::bail!("unknown_operation"),
            }
        } else if path.starts_with("/source/") {
            let source = auth(&db, &h, "source")?;
            if path.starts_with("/source/desktop-") {
                // Independent desktop management polls must not replace the
                // source owner's session or invalidate its freeze barrier.
                anyhow::ensure!(method == Method::POST, "method_not_allowed");
                return match path {
                    "/source/desktop-announce" => db.desktop_announce(&source,&v),
                    "/source/desktop-status" => db.desktop_status(&source.device_id),
                    "/source/desktop-request" => db.desktop_request(&source.device_id,v),
                    "/source/desktop-next" => db.desktop_next(&source.device_id),
                    "/source/desktop-authorize" => db.desktop_authorize(&source.device_id,field(&v,"id")),
                    "/source/desktop-respond" => db.desktop_respond(&source.device_id,v),
                    "/source/desktop-result" => db.desktop_result(&source.device_id,field(&v,"id")),
                    _ => anyhow::bail!("unknown_operation"),
                };
            }
            // Desktop management requests share the source credential but not
            // the long-running transport session or its freeze barrier epoch.
            if matches!(
                path,
                "/source/pairing" | "/source/pairing-session" | "/source/pairing-applied"
            ) {
                anyhow::ensure!(method == Method::POST, "method_not_allowed");
                return match path {
                    "/source/pairing" => db.source_pairing(&source),
                    "/source/pairing-session" => {
                        db.source_pairing_session(&source, field(&v, "session_id"))
                    }
                    _ => {
                        db.source_pairing_applied(&source, &v)?;
                        Ok(json!({"ok":true}))
                    }
                };
            }
            db.touch_session(&source.device_id, header(&h, "x-iterate-source-session"))?;
            if path == "/source/next" && method == Method::GET {
                return db.next(&source.device_id);
            }
            anyhow::ensure!(method == Method::POST, "method_not_allowed");
            match path {
                "/source/publish" => db.publish(&source.device_id, serde_json::from_value(v)?)?,
                "/source/close" => db.close_request(&source.device_id, v)?,
                "/source/confirm" => {
                    return db.confirm(&source.device_id, serde_json::from_value(v)?)
                }
                "/source/respond" => db.rpc_response(&source.device_id, v)?,
                "/source/image" => return db.image(&source.device_id, v),
                "/source/barrier" => db.barrier(&source.device_id, v)?,
                "/source/heartbeat" => db.touch(&source.device_id)?,
                "/source/authorize" => {
                    return db.authorize_job(&source.device_id, field(&v, "job_id"))
                }
                _ => anyhow::bail!("unknown_operation"),
            }
        } else if path == "/api/android/pairing/claim" {
            anyhow::ensure!(method == Method::POST, "method_not_allowed");
            return db.claim(v);
        } else {
            let phone = auth(&db, &h, "phone")?;
            anyhow::ensure!(
                method == Method::GET && path == "/api/android/pairing/status",
                "method_not_allowed"
            );
            return Ok(
                json!({"ok":true,"device_id":phone.device_id,"client_kind":phone.client_kind,"scopes":phone.scopes}),
            );
        }
        Ok(json!({"ok":true}))
    })();
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => failure(e),
    }
}
async fn image(State(hub): State<Hub>, uri: Uri, h: HeaderMap) -> Response {
    let result = (|| -> anyhow::Result<(String, Vec<u8>)> {
        let db = hub
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store_unavailable"))?;
        let phone = auth(&db, &h, "phone")?;
        let q = uri.query().unwrap_or("");
        anyhow::ensure!(
            q.starts_with("id=") && !q.contains('&'),
            "invalid_image_query"
        );
        db.read_image(&phone, &q[3..])
    })();
    match result {
        Ok((mime, data)) => (
            [
                ("content-type", mime),
                ("cache-control", "private, no-store".into()),
                ("x-content-type-options", "nosniff".into()),
            ],
            data,
        )
            .into_response(),
        Err(e) => failure(e),
    }
}
async fn open_codex(State(hub): State<Hub>, method: Method, h: HeaderMap, body: Bytes) -> Response {
    let prepare = (|| -> anyhow::Result<String> {
        anyhow::ensure!(
            method == Method::POST && body.len() <= MAX_MESSAGE,
            "invalid_request"
        );
        let v: Value = serde_json::from_slice(&body)?;
        let mut db = hub
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store_unavailable"))?;
        let phone = auth(&db, &h, "phone")?;
        db.rpc(&phone, "open_codex", v)
    })();
    let id = match prepare {
        Ok(id) => id,
        Err(e) => return failure(e),
    };
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let result = (|| -> anyhow::Result<Option<Value>> {
            let db = hub
                .store
                .lock()
                .map_err(|_| anyhow::anyhow!("store_unavailable"))?;
            let phone = auth(&db, &h, "phone")?;
            db.rpc_result(&phone, &id)
        })();
        match result {
            Ok(Some(v)) => return Json(v).into_response(),
            Ok(None) => {}
            Err(e) => return failure(e),
        }
    }
    (
        StatusCode::GATEWAY_TIMEOUT,
        Json(json!({"ok":false,"error":"source_confirmation_unknown"})),
    )
        .into_response()
}
async fn main_page(State(hub): State<Hub>, uri: Uri, h: HeaderMap) -> Response {
    let prepare = (|| -> anyhow::Result<String> {
        let mut db = hub
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("store_unavailable"))?;
        let phone = auth(&db, &h, "phone")?;
        db.rpc(&phone, "page", json!({"method":"GET","path":uri.path()}))
    })();
    let id = match prepare {
        Ok(id) => id,
        Err(e) => return failure(e),
    };
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let result = (|| -> anyhow::Result<Option<Value>> {
            let db = hub
                .store
                .lock()
                .map_err(|_| anyhow::anyhow!("store_unavailable"))?;
            let phone = auth(&db, &h, "phone")?;
            db.rpc_result(&phone, &id)
        })();
        match result {
            Ok(Some(v)) => {
                let status = StatusCode::from_u16(
                    v["status"]
                        .as_u64()
                        .unwrap_or(503)
                        .try_into()
                        .unwrap_or(503),
                )
                .unwrap_or(StatusCode::SERVICE_UNAVAILABLE);
                return (
                    status,
                    [
                        ("content-type", "text/html; charset=utf-8"),
                        ("cache-control", "private, no-store"),
                    ],
                    field(&v, "body").to_string(),
                )
                    .into_response();
            }
            Ok(None) => {}
            Err(e) => return failure(e),
        }
    }
    (StatusCode::GATEWAY_TIMEOUT, "Windows content unavailable").into_response()
}
async fn upgrade(State(hub): State<Hub>, h: HeaderMap, ws: WebSocketUpgrade) -> Response {
    let check = hub
        .store
        .lock()
        .map_err(|_| anyhow::anyhow!("store_unavailable"))
        .and_then(|db| auth(&db, &h, "phone"));
    match check {
        Ok(_) => ws
            .max_message_size(MAX_MESSAGE)
            .max_frame_size(MAX_MESSAGE)
            .on_upgrade(move |socket| phone_socket(hub, h, socket)),
        Err(e) => failure(e),
    }
}
async fn phone_socket(hub: Hub, h: HeaderMap, mut socket: WebSocket) {
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    let mut seen = HashSet::new();
    let mut sync_reason: Option<String> = None;
    let session = uuid::Uuid::new_v4().to_string();
    loop {
        tokio::select! {
            _=tick.tick()=>{
                let result=(||->anyhow::Result<Vec<Value>>{let db=hub.store.lock().map_err(|_|anyhow::anyhow!("store_unavailable"))?;let phone=auth(&db,&h,"phone")?;
                    if !phone.scope("session.read"){return Ok(Vec::new())}db.snapshots_for_session(&phone,sync_reason.as_deref(),&session)})();
                match result{
                    Err(_)=>{let _=socket.send(Message::Close(None)).await;break;},
                    Ok(events)=>{let mut current=HashSet::new();for event in events{let text=event.to_string();let digest=hash(text.as_bytes());if current.insert(digest.clone())&&!seen.contains(&digest)&&socket.send(Message::Text(text)).await.is_err(){return;}}
                        // No event ACK is inferred. Reconnecting clients receive the
                        // durable current snapshots and their original action states.
                        // Keep only the current view, not connection history:
                        // active -> offline/closed -> active must be delivered.
                        seen=current;
                    }
                }
            },
            incoming=socket.recv()=>{
                let text=match incoming{Some(Ok(Message::Text(t)))=>t,Some(Ok(Message::Close(_)))|None|Some(Err(_))=>break,_=>continue};
                let wire:Value=match serde_json::from_str(&text){Ok(v)=>v,Err(_)=>{let _=socket.send(Message::Close(None)).await;break;}};
                let result=(||->anyhow::Result<Option<Value>>{
                    let mut db=hub.store.lock().map_err(|_|anyhow::anyhow!("store_unavailable"))?;let phone=auth(&db,&h,"phone")?;
                    match field(&wire,"message_type"){
                        "client_hello"=>{anyhow::ensure!(field(&wire["payload"],"device_id")==phone.device_id&&field(&wire["payload"],"client_kind")==phone.client_kind,"hello_identity_mismatch");db.pairing_connected(&phone,field(&wire["payload"],"selected_transport_mode"))?;Ok(None)},
                        "mcp_action"|"cross_device_mirror_action"=>Ok(Some(db.action(&phone,wire.clone()).unwrap_or_else(|e|{
                            let reason=safe_reason(&e);
                            if matches!(reason.as_str(),"scope_denied"|"message_too_large"|"invalid_action"|"action_content_conflict"|"source_permission_denied"|"unknown_action_query"|"route_frozen"|"unknown_request"|"source_revoked"|"unknown_source"|"stale_version_or_closed"|"project_mismatch"|"mirror_identity_mismatch"|"mirror_action_forbidden"|"action_kind_mismatch"|"capacity_exceeded") {denied(&wire,&reason)}else{unknown(&wire)}
                        }))),
                        "request_sync"=>{
                            sync_reason=Some(field(&wire["payload"],"sync_reason").chars().take(256).collect());seen.clear();
                            // Windows owns templates/native history/main content;
                            // authorized Mac published mirrors come from hub snapshots.
                            match db.rpc_for_session(&phone,"sync",wire.clone(),&session){Ok(_)=>Ok(None),Err(e)=>Ok(Some(json!({"message_type":"hub_status","payload":{"windows_available":false,"reason":safe_reason(&e)}})))}
                        },
                        "request_timeline_sync"=>{
                            match db.rpc_for_session(&phone,"timeline",wire.clone(),&session){
                                Ok(_)=>Ok(None),
                                Err(e) if e.to_string()=="windows_offline"=>Ok(Some(json!({"message_type":"hub_status","payload":{"windows_available":false,"reason":"windows_offline"}}))),
                                Err(e)=>Err(e)
                            }
                        },
                        _=>anyhow::bail!("unsupported_message")
                    }
                })();
                match result{Ok(Some(reply))=>{if socket.send(Message::Text(reply.to_string())).await.is_err(){break;}},Ok(None)=>{},Err(_)=>{let _=socket.send(Message::Close(None)).await;break;}}
            }
        }
    }
}
fn safe_reason(e: &anyhow::Error) -> String {
    let s = e.to_string();
    if s.len() <= 96 && s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
        s
    } else {
        "request_failed".into()
    }
}
