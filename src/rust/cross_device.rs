//! Paired desktop transport. TLS authenticates each configured device;
//! the peer protocol cannot choose local filenames, executables or arguments.
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Arc,
    time::Duration,
};

const VERSION: u32 = 2;
type Result<T> = std::result::Result<T, String>;

fn directory() -> Result<PathBuf> {
    match std::env::var_os("ITERATE_CROSS_DEVICE_DIR") {
        Some(path) => Ok(PathBuf::from(path)),
        None => crate::config::cunzhi_config_dir()
            .map(|path| path.join("cross-device"))
            .map_err(|error| error.to_string()),
    }
}

pub mod transport;
pub mod settings_sync;
use transport::{connection_config, credential, local_daemon_ready, peer};

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().ok_or("缺少目录")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temp.write_all(&serde_json::to_vec(value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

fn lock(dir: &Path, name: &str) -> Result<File> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(dir.join(name))
        .map_err(|e| e.to_string())?;
    file.lock().map_err(|e| e.to_string())?;
    Ok(file)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

#[derive(Clone, Serialize, Deserialize)]
struct Settings {
    device_id: String,
    enabled: bool,
}

fn settings() -> Result<Settings> {
    let dir = directory()?;
    let _guard = lock(&dir, "settings.lock")?;
    let path = dir.join("settings.json");
    if path.exists() {
        return read_json(&path);
    }
    let value = Settings {
        device_id: uuid::Uuid::new_v4().to_string(),
        enabled: false,
    };
    atomic_json(&path, &value)?;
    Ok(value)
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Registration {
    key: String,
    origin_device_id: String,
    origin_name: String,
    request: Value,
    request_file: PathBuf,
    response_file: PathBuf,
}

impl Registration {
    fn path(&self) -> Result<PathBuf> {
        Ok(directory()?
            .join("requests")
            .join(format!("{}.json", self.key)))
    }
    fn result_path(&self) -> Result<PathBuf> {
        Ok(directory()?
            .join("results")
            .join(format!("{}.json", self.key)))
    }
    fn request_lock(&self) -> Result<File> {
        lock(&directory()?.join("locks"), &format!("{}.lock", self.key))
    }
    fn active(&self) -> bool {
        self.pending() && self.result_path().is_ok_and(|p| !p.exists())
    }
    fn pending(&self) -> bool {
        let fresh = directory()
            .ok()
            .and_then(|d| fs::metadata(d.join("leases").join(&self.key)).ok())
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age < Duration::from_secs(15));
        fresh && self.request_file.is_file()
    }
    pub fn renew(&self) {
        if let Ok(dir) = directory() {
            let path = dir.join("leases").join(&self.key);
            let recent = fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age < Duration::from_secs(1));
            if !recent {
                let _ = fs::create_dir_all(dir.join("leases"));
                let _ = fs::write(path, b"active");
            }
        }
    }
    pub fn key(&self) -> &str {
        &self.key
    }
    pub fn recover_accepted_response(&self) {
        // If the source window exits after the peer won, preserve that accepted
        // reply instead of turning it into a cancellation. The receipt remains.
        if let Ok(_guard) = self.request_lock() {
            if !self.response_file.exists() {
                if let Ok(path) = self.result_path() {
                    if let Ok(receipt) = read_json::<Value>(&path) {
                        if let Some(response) = receipt.get("response") {
                            let _ = atomic_json(&self.response_file, response);
                        }
                    } else {
                        // Closing acceptance and inspecting the winner are one
                        // transaction; a late peer cannot win after this point.
                        let _ = atomic_json(&path, &json!({"cancelled":true}));
                    }
                }
            }
        }
    }
    pub fn finish(&self) {
        if let Ok(_guard) = self.request_lock() {
            if let Ok(path) = self.result_path() {
                if !path.exists() {
                    let _ = atomic_json(&path, &json!({"cancelled": true}));
                }
            }
        }
    }
}

fn has_attachments(value: &Value) -> bool {
    ["images", "image_paths", "file_paths", "attachments"]
        .iter()
        .any(|k| {
            value
                .get(k)
                .is_some_and(|v| v.as_array().is_some_and(|a| !a.is_empty()))
        })
}

pub async fn register(
    request: &Value,
    request_file: &Path,
    response_file: &Path,
) -> Option<Registration> {
    let route_key = transport::route_key().ok()?;
    let local = settings().ok()?;
    if !local.enabled || std::env::var_os("ITERATE_CROSS_DEVICE_MIRROR").is_some() {
        return None;
    }
    if transport::ensure_daemon().await.is_err() {
        return None;
    }
    let message = request.get("message").and_then(Value::as_str).unwrap_or("");
    if has_attachments(request) || message.contains("![") || message.contains("<img") {
        return None;
    }
    let snapshot = peer("/snapshot", None).await.ok()?;
    if snapshot.get("enabled").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let _route_guard = lock(&directory().ok()?, "connection.lock").ok()?;
    if transport::route_key().ok()? != route_key {
        return None;
    }
    let registration = Registration {
        key: uuid::Uuid::new_v4().to_string(),
        origin_device_id: local.device_id,
        origin_name: connection_config()
            .ok()
            .map(|c| c.device_name)
            .unwrap_or_else(|| std::env::consts::OS.into()),
        request: request.clone(),
        request_file: request_file.into(),
        response_file: response_file.into(),
    };
    atomic_json(&registration.path().ok()?, &registration).ok()?;
    registration.renew();
    Some(registration)
}

fn load_registration(key: &str) -> Result<Registration> {
    uuid::Uuid::parse_str(key).map_err(|_| "无效请求标识")?;
    read_json(&directory()?.join("requests").join(format!("{key}.json")))
}

// The same OS file lock is used by the local popup and peer HTTP handler.
// Atomic result persistence makes retry safe even when the ACK is lost.
fn commit(reg: &Registration, response: &Value, remote: bool) -> Result<()> {
    let _guard = reg.request_lock()?;
    let result_path = reg.result_path()?;
    if result_path.exists() {
        let previous: Value = read_json(&result_path)?;
        if previous.get("response") == Some(response) {
            return Ok(());
        }
        return Err("该请求已由另一端完成或关闭".into());
    }
    if !reg.active() {
        return Err("源请求已结束".into());
    }
    if remote && has_attachments(response) {
        return Err("跨设备 spike 仅支持文本和选项，请在源端发送附件".into());
    }
    // Persist the winner before the source GUI performs its existing recording
    // and response-file publication. A killed GUI cannot lose this receipt.
    atomic_json(&result_path, &json!({"response": response}))?;
    Ok(())
}

pub async fn submit(response: &Value) -> Result<bool> {
    if let Ok(key) = std::env::var("ITERATE_CROSS_DEVICE_MIRROR") {
        if response.as_str() == Some("CANCELLED")
            || response.pointer("/metadata/source").and_then(Value::as_str) == Some("popup_closed")
        {
            return Ok(true);
        }
        if has_attachments(response) {
            return Err("镜像仅支持文本和选项，请在源设备发送附件".into());
        }
        let result = peer("/submit", Some(&json!({"key":key,"response":response}))).await?;
        if result.get("ok").and_then(Value::as_bool) != Some(true) {
            return Err(result
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("发送未确认")
                .into());
        }
        return Ok(true);
    }
    if let Ok(key) = std::env::var("ITERATE_CROSS_DEVICE_SOURCE") {
        if key.is_empty() {
            return Ok(false);
        }
        commit(&load_registration(&key)?, response, false)?;
        return Ok(false);
    }
    Ok(false)
}

pub fn source_finalize_guard() -> Result<Option<File>> {
    let Ok(key) = std::env::var("ITERATE_CROSS_DEVICE_SOURCE") else {
        return Ok(None);
    };
    if key.is_empty() {
        return Ok(None);
    }
    let reg = load_registration(&key)?;
    let dir = directory()?.join("finalizing");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(dir.join(key))
        .map_err(|e| e.to_string())?;
    file.try_lock().map_err(|_| "获胜回复正在处理中")?;
    if reg.response_file.exists() {
        return Err("回复已完成".into());
    }
    Ok(Some(file))
}

pub fn publish_source_response(path: &Path, response: &Value) -> Result<()> {
    atomic_json(path, response)
}

#[tauri::command]
pub async fn get_cross_device_status() -> Value {
    let mirror = std::env::var_os("ITERATE_CROSS_DEVICE_MIRROR").is_some();
    let local = match settings() {
        Ok(s) => s,
        Err(error) => {
            return json!({"enabled":false,"connected":false,"mirror":mirror,"error":error})
        }
    };
    let remote = peer("/snapshot", None).await;
    let local_ready = local_daemon_ready().await;
    let mirror_key = std::env::var("ITERATE_CROSS_DEVICE_MIRROR").ok();
    let local_only =
        std::env::var("ITERATE_CROSS_DEVICE_SOURCE").is_ok_and(|key| key.is_empty()) && !mirror;
    let source_pending = std::env::var("ITERATE_CROSS_DEVICE_SOURCE").ok().filter(|key|!key.is_empty())
        .and_then(|key|load_registration(&key).ok()).filter(|reg|!reg.response_file.exists())
        .and_then(|reg|read_json::<Value>(&reg.result_path().ok()?).ok().and_then(|receipt|receipt.get("response").cloned())
            .map(|response|json!({"response":response,"request_id":reg.request.get("id"),"project_path":reg.request.get("project_path")})));
    let resolved = mirror_key.as_ref().is_some_and(|key| {
        remote
            .as_ref()
            .ok()
            .and_then(|v| v.get("requests"))
            .and_then(Value::as_array)
            .is_some_and(|items| {
                !items
                    .iter()
                    .any(|r| r.get("key").and_then(Value::as_str) == Some(key.as_str()))
            })
    });
    let origin_name = if mirror {
        std::env::var("ITERATE_CROSS_DEVICE_ORIGIN_NAME").unwrap_or_else(|_| "另一设备".into())
    } else {
        connection_config()
            .map(|c| c.device_name)
            .unwrap_or_else(|_| std::env::consts::OS.into())
    };
    json!({"enabled":local.enabled,"device_id":local.device_id,"origin_name":origin_name,"connected":remote.is_ok() && local_ready,"resolved":resolved,"local_only":local_only,"source_pending":source_pending,
        "connected_ip":remote.as_ref().ok().and_then(|v|v.get("connected_ip")),"using_backup":remote.as_ref().ok().and_then(|v|v.get("using_backup")),
        "peer_enabled":remote.as_ref().ok().and_then(|v|v.get("enabled")).and_then(Value::as_bool).unwrap_or(false),
        "mirror":mirror,"error":remote.err(),"text_only":true})
}

#[tauri::command]
pub async fn set_cross_device_enabled(enabled: bool) -> Result<Value> {
    if enabled {
        transport::validate_enable()?;
        transport::ensure_daemon().await?;
    }
    let mut current = settings()?;
    let dir = directory()?;
    {
        let _guard = lock(&dir, "settings.lock")?;
        current.enabled = enabled;
        atomic_json(&dir.join("settings.json"), &current)?;
    }
    Ok(get_cross_device_status().await)
}

struct Broker {
    token: String,
    revision: String,
    device_id: String,
    network_ready: std::sync::atomic::AtomicBool,
}
fn authorized(headers: &HeaderMap, state: &Broker) -> bool {
    headers.get("authorization").is_some_and(|header| {
        ring::constant_time::verify_slices_are_equal(
            header.as_bytes(),
            format!("Bearer {}", state.token).as_bytes(),
        )
        .is_ok()
    })
}

async fn settings_snapshot(
    State(state): State<Arc<Broker>>,
    headers: HeaderMap,
    Json(request): Json<settings_sync::ExportRequest>,
) -> std::result::Result<Json<Value>, StatusCode> {
    if !authorized(&headers, &state) { return Err(StatusCode::UNAUTHORIZED); }
    if transport::route_key().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)? != state.revision {
        return Err(StatusCode::CONFLICT);
    }
    let snapshot = settings_sync::settings_sync_export(request.categories)
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    let bytes = settings_sync::bounded_bytes(&snapshot).map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;
    serde_json::from_slice(&bytes).map(Json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn snapshot(
    State(state): State<Arc<Broker>>,
    headers: HeaderMap,
) -> std::result::Result<Json<Value>, StatusCode> {
    if !authorized(&headers, &state) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let config = settings().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut requests = Vec::new();
    if let Ok(entries) = fs::read_dir(
        directory()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .join("requests"),
    ) {
        for entry in entries.flatten() {
            if let Ok(reg) = read_json::<Registration>(&entry.path()) {
                if reg.active() {
                    requests.push(
                        json!({"key":reg.key,"origin_device_id":reg.origin_device_id,
                    "origin_name":reg.origin_name,"request":reg.request}),
                    );
                }
            }
        }
    }
    Ok(Json(
        json!({"version":VERSION,"config_revision":state.revision,"enabled":config.enabled,"device_id":config.device_id,"device_name":connection_config().ok().map(|c|c.device_name),"requests":requests}),
    ))
}

// Local readiness must not traverse a VPN interface: peer-only WireGuard routes
// can reach the other device while rejecting traffic to this device's own IP.
async fn health(
    State(state): State<Arc<Broker>>,
    headers: HeaderMap,
) -> std::result::Result<Json<Value>, StatusCode> {
    if !authorized(&headers, &state) { return Err(StatusCode::UNAUTHORIZED); }
    if !state.network_ready.load(std::sync::atomic::Ordering::Acquire) {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    Ok(Json(json!({"version":VERSION,"config_revision":state.revision,"device_id":state.device_id})))
}

pub fn mirror_process_guard() -> Result<Option<File>> {
    let Ok(key) = std::env::var("ITERATE_CROSS_DEVICE_MIRROR") else {
        return Ok(None);
    };
    uuid::Uuid::parse_str(&key).map_err(|_| "无效镜像标识")?;
    let dir = directory()?.join("mirror-locks");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(dir.join(key))
        .map_err(|e| e.to_string())?;
    file.try_lock().map_err(|_| "该镜像窗口已存在")?;
    Ok(Some(file))
}

async fn submit_peer(
    State(state): State<Arc<Broker>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> std::result::Result<Json<Value>, StatusCode> {
    if !authorized(&headers, &state) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let result = (|| {
        let reg = load_registration(
            body.get("key")
                .and_then(Value::as_str)
                .ok_or("缺少请求标识")?,
        )?;
        let response = body.get("response").ok_or("缺少响应")?;
        if !response.is_object() {
            return Err("无效响应".into());
        }
        commit(&reg, response, true)
    })();
    Ok(Json(match result {
        Ok(()) => {
            json!({"version":VERSION,"device_id":settings().ok().map(|s|s.device_id),"ok":true})
        }
        Err(error) => {
            json!({"version":VERSION,"device_id":settings().ok().map(|s|s.device_id),"ok":false,"error":error})
        }
    }))
}

struct Mirror {
    child: Child,
    request_file: PathBuf,
    ready_file: PathBuf,
}
impl Drop for Mirror {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_file(&self.request_file);
        let _ = fs::remove_file(&self.ready_file);
        if let (Ok(dir), Some(key)) = (directory(), self.request_file.file_stem()) {
            let _ = fs::remove_file(dir.join("mirror-leases").join(key));
        }
    }
}

fn renew_mirror_lease(key: &str) {
    if let Ok(dir) = directory() {
        let folder = dir.join("mirror-leases");
        let _ = fs::create_dir_all(&folder);
        let _ = fs::write(folder.join(key), b"active");
    }
}

fn spawn_mirror(item: &Value) -> Result<Mirror> {
    let key = item
        .get("key")
        .and_then(Value::as_str)
        .ok_or("缺少请求标识")?;
    uuid::Uuid::parse_str(key).map_err(|_| "无效请求标识")?;
    let source = item.get("request").ok_or("缺少请求")?;
    let name = item
        .get("origin_name")
        .and_then(Value::as_str)
        .unwrap_or("另一设备");
    // Explicit allowlist: never import paths, checkpoint IDs, deeplinks or flags.
    let text = source.get("message").and_then(Value::as_str).unwrap_or("");
    let plain_links = regex::Regex::new(r"\[([^\]]*)\]\([^)]*\)")
        .map_err(|e| e.to_string())?
        .replace_all(text, "$1");
    let request = json!({"id":format!("cross-{key}"),"message":format!("跨设备来源：{name} · 仅支持文本和选项\n\n{plain_links}"),
        "predefined_options":source.get("predefined_options").cloned().unwrap_or(json!([])),
        "is_markdown":source.get("is_markdown").cloned().unwrap_or(json!(true)),
        "conversation_title":format!("来自 {name} · {}",source.get("conversation_title").and_then(Value::as_str).unwrap_or("跨设备请求"))});
    let dir = directory()?.join("mirrors");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let request_file = dir.join(format!("{key}.json"));
    let ready_file = dir.join(format!("{key}.ready"));
    atomic_json(&request_file, &request)?;
    let executable = std::env::var_os("ITERATE_DIALOG_GUI_EXECUTABLE")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_exe().map_err(|e| e.to_string())?);
    let mut command = Command::new(executable);
    command
        .env("ITERATE_MCP_REQUEST_FILE", &request_file)
        .env("ITERATE_READY_FILE", &ready_file)
        .env("ITERATE_STANDALONE_MODE", "1")
        .env("ITERATE_CROSS_DEVICE_MIRROR", key)
        .env("ITERATE_CROSS_DEVICE_ORIGIN_NAME", name)
        .env_remove("ITERATE_RESPONSE_FILE")
        .env_remove("ITERATE_DELIVERY_FILE")
        .env_remove("ITERATE_CROSS_DEVICE_SOURCE")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let child = command.spawn().map_err(|e| e.to_string())?;
    renew_mirror_lease(key);
    Ok(Mirror {
        child,
        request_file,
        ready_file,
    })
}

async fn mirror_loop() {
    let mut mirrors: HashMap<String, Mirror> = HashMap::new();
    let mut dismissed: HashSet<String> = HashSet::new();
    loop {
        let route_key = transport::route_key().ok();
        if let Ok(snapshot) = peer("/snapshot", None).await {
            let Ok(dir) = directory() else {
                continue;
            };
            let Ok(_route_guard) = lock(&dir, "connection.lock") else {
                continue;
            };
            if transport::route_key().ok() != route_key {
                continue;
            }
            let requests = snapshot
                .get("requests")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let active: HashSet<String> = requests
                .iter()
                .filter_map(|r| r.get("key").and_then(Value::as_str).map(str::to_string))
                .collect();
            mirrors.retain(|key, mirror| {
                if !active.contains(key) {
                    return false;
                }
                if mirror.child.try_wait().ok().flatten().is_some() {
                    dismissed.insert(key.clone());
                    if let Ok(dir) = directory() {
                        let _ = atomic_json(&dir.join("dismissed").join(key), &json!(true));
                    }
                    return false;
                }
                renew_mirror_lease(key);
                true
            });
            dismissed.retain(|key| active.contains(key));
            if settings().is_ok_and(|s| s.enabled) {
                for item in requests {
                    let Some(key) = item.get("key").and_then(Value::as_str) else {
                        continue;
                    };
                    if directory().is_ok_and(|dir| dir.join("dismissed").join(key).exists()) {
                        continue;
                    }
                    if !mirrors.contains_key(key) && !dismissed.contains(key) {
                        match spawn_mirror(&item) {
                            Ok(mirror) => {
                                mirrors.insert(key.into(), mirror);
                            }
                            Err(e) => eprintln!("cross-device popup: {e}"),
                        }
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

pub fn run_daemon(_port: u16) -> anyhow::Result<()> {
    let dir = directory().map_err(anyhow::Error::msg)?;
    let owner = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(dir.join("daemon.lock"))?;
    owner
        .try_lock()
        .map_err(|_| anyhow::anyhow!("跨设备服务已经运行"))?;
    atomic_json(
        &dir.join("daemon-process.json"),
        &json!({"pid":std::process::id()}),
    )
    .map_err(anyhow::Error::msg)?;
    let _ = rustls::crypto::ring::default_provider().install_default();
    tokio::runtime::Runtime::new()?.block_on(async move {
        let task = tokio::spawn(mirror_loop());
        loop {
            if !dir.join("direct-connection.json").is_file() { task.abort(); return Ok::<(), std::io::Error>(()); }
            let revision = transport::route_key().map_err(std::io::Error::other)?;
            let tls = transport::server_tls().map_err(std::io::Error::other)?;
            let state = Arc::new(Broker {
                token: credential().map_err(std::io::Error::other)?,
                revision: revision.clone(),
                device_id: settings().map_err(std::io::Error::other)?.device_id,
                network_ready: std::sync::atomic::AtomicBool::new(false),
            });
            let config = connection_config().map_err(std::io::Error::other)?;
            let app = Router::new()
                .route("/snapshot", get(snapshot))
                .route("/submit", post(submit_peer))
                .route("/api/settings-sync/snapshot", post(settings_snapshot)
                    .layer(DefaultBodyLimit::max(settings_sync::MAX_REQUEST_BYTES)))
                .layer(DefaultBodyLimit::max(1024 * 1024))
                .with_state(state.clone());
            // An OS-selected loopback port avoids collisions between local profiles.
            // It exposes only authenticated health, not the request/response routes.
            let health_listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
            let health_port = health_listener.local_addr()?.port();
            let health_handle = axum_server::Handle::new();
            let health_acceptor = axum_server::tls_rustls::RustlsAcceptor::new(tls.clone())
                .handshake_timeout(Duration::from_secs(5)).acceptor(transport::LimitedAcceptor::new());
            let health_app = Router::new().route("/health", get(health)).with_state(state.clone());
            let health_server = tokio::spawn(axum_server::from_tcp(health_listener)
                .acceptor(health_acceptor).handle(health_handle.clone()).serve(health_app.into_make_service()));
            atomic_json(&dir.join("daemon-process.json"), &json!({"pid":std::process::id(),"health_port":health_port}))
                .map_err(std::io::Error::other)?;
            let mut listeners: HashMap<std::net::Ipv4Addr, (axum_server::Handle, tokio::task::JoinHandle<std::io::Result<()>>)> = HashMap::new();
            let mut next_network_check = std::time::Instant::now();
            loop {
                if std::time::Instant::now() >= next_network_check {
                    let available = transport::local_ips(&config.listen_ip).map_err(std::io::Error::other)?;
                    listeners.retain(|ip, (handle, server)| {
                        if !available.contains(ip) || server.is_finished() {
                            handle.shutdown();
                            server.abort();
                            false
                        } else { true }
                    });
                    for ip in available {
                        if listeners.contains_key(&ip) { continue; }
                        // Missing interfaces/public NAT addresses are never bound as
                        // wildcard listeners. Retry new interfaces after network changes.
                        if let Ok(listener) = std::net::TcpListener::bind((ip, config.listen_port)) {
                            let handle = axum_server::Handle::new();
                            let acceptor = axum_server::tls_rustls::RustlsAcceptor::new(tls.clone())
                                .handshake_timeout(Duration::from_secs(5)).acceptor(transport::LimitedAcceptor::new());
                            let server = axum_server::from_tcp(listener).acceptor(acceptor)
                                .handle(handle.clone()).serve(app.clone().into_make_service());
                            let server = tokio::spawn(server);
                            if tokio::time::timeout(Duration::from_secs(1), handle.listening()).await.ok().flatten().is_some() {
                                listeners.insert(ip, (handle, server));
                            } else {
                                handle.shutdown();
                                server.abort();
                            }
                        }
                    }
                    next_network_check = std::time::Instant::now() + Duration::from_secs(2);
                }
                state.network_ready.store(!listeners.is_empty(), std::sync::atomic::Ordering::Release);
                if !dir.join("direct-connection.json").is_file() || transport::route_key().map_err(std::io::Error::other)? != revision {
                    state.network_ready.store(false, std::sync::atomic::Ordering::Release);
                    health_handle.graceful_shutdown(Some(Duration::from_millis(300)));
                    for (handle, _) in listeners.values() {
                        handle.graceful_shutdown(Some(Duration::from_millis(300)));
                    }
                    tokio::time::sleep(Duration::from_millis(350)).await;
                    for (_, server) in listeners.into_values() { server.abort(); }
                    health_server.abort();
                    break;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn local_health_requires_authentication_and_a_network_listener() {
        let state = Arc::new(Broker { token: "test-token".into(), revision: "test-revision".into(),
            device_id: "test-device".into(), network_ready: std::sync::atomic::AtomicBool::new(false) });
        assert_eq!(health(State(state.clone()), HeaderMap::new()).await.unwrap_err(), StatusCode::UNAUTHORIZED);
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer test-token".parse().unwrap());
        assert_eq!(health(State(state.clone()), headers.clone()).await.unwrap_err(), StatusCode::SERVICE_UNAVAILABLE);
        state.network_ready.store(true, std::sync::atomic::Ordering::Release);
        let value = health(State(state), headers).await.unwrap().0;
        assert_eq!(value["config_revision"], "test-revision");
        assert_eq!(value["device_id"], "test-device");
        assert!(value.get("requests").is_none());
    }
    #[test]
    fn first_submit_is_atomic_retryable_and_cancel_is_final() {
        let temp = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("ITERATE_CROSS_DEVICE_DIR");
        std::env::set_var("ITERATE_CROSS_DEVICE_DIR", temp.path());
        let request_file = temp.path().join("request.json");
        fs::write(&request_file, b"{}").unwrap();
        let reg = Registration {
            key: uuid::Uuid::new_v4().to_string(),
            origin_device_id: "origin".into(),
            origin_name: "test".into(),
            request: json!({}),
            request_file,
            response_file: temp.path().join("response.json"),
        };
        reg.renew();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let handles: Vec<_> = (0..2)
            .map(|i| {
                let reg = reg.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let response = json!({"user_input":format!("winner-{i}")});
                    (commit(&reg, &response, true), response)
                })
            })
            .collect();
        barrier.wait();
        let attempts: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(attempts.iter().filter(|(r, _)| r.is_ok()).count(), 1);
        let winner = &attempts.iter().find(|(r, _)| r.is_ok()).unwrap().1;
        assert!(
            reg.pending(),
            "accepted replies still block pairing changes until source delivery"
        );
        assert!(!reg.active());
        assert_eq!(
            read_json::<Value>(&reg.result_path().unwrap())
                .unwrap()
                .get("response"),
            Some(winner)
        );
        assert!(
            !reg.response_file.exists(),
            "only the source GUI may publish after recording"
        );
        assert!(
            commit(&reg, winner, true).is_ok(),
            "lost ACK retry must succeed"
        );
        reg.finish();
        assert!(!reg.active());
        let cancelled = Registration {
            key: uuid::Uuid::new_v4().to_string(),
            response_file: temp.path().join("cancelled-response.json"),
            ..reg.clone()
        };
        cancelled.renew();
        cancelled.finish();
        assert!(commit(&cancelled, &json!({"user_input":"late"}), true).is_err());
        assert!(!cancelled.response_file.exists());
        let exiting = Registration {
            key: uuid::Uuid::new_v4().to_string(),
            response_file: temp.path().join("exit-race.json"),
            ..reg.clone()
        };
        exiting.renew();
        let exit_barrier = Arc::new(std::sync::Barrier::new(2));
        let peer_reg = exiting.clone();
        let peer_barrier = exit_barrier.clone();
        let peer_submit = std::thread::spawn(move || {
            peer_barrier.wait();
            commit(&peer_reg, &json!({"user_input":"exit-race"}), true)
        });
        exit_barrier.wait();
        exiting.recover_accepted_response();
        let accepted = peer_submit.join().unwrap().is_ok();
        assert_eq!(
            accepted,
            exiting.response_file.exists(),
            "an accepted peer reply must survive source exit"
        );
        match previous {
            Some(value) => std::env::set_var("ITERATE_CROSS_DEVICE_DIR", value),
            None => std::env::remove_var("ITERATE_CROSS_DEVICE_DIR"),
        }
    }
}
