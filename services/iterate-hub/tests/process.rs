use futures_util::{SinkExt, StreamExt};
use iterate_hub::protocol::hash;
use serde_json::{json, Value};
use std::{
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

const ADMIN: &str = "temporary-admin-process-test-0123456789";

#[tokio::test]
async fn desktop_relay_over_real_http_preserves_owner_session_and_source_ack() {
    let (_dir,_running,client,base)=regression_fixture().await;
    let before=source_next(&client,&base,"win","windows").await;
    for (id,kind,own,peer) in [("win","windows",b"win-pair".as_slice(),b"mac-pair".as_slice()),
        ("mac","macos",b"mac-pair".as_slice(),b"win-pair".as_slice())] {
        headers(client.post(format!("{base}/source/desktop-announce")),id,kind)
            .header("X-Iterate-Source-Session","independent-desktop-status")
            .json(&json!({"own_hash":hash(own),"peer_hash":hash(peer),"name":id,"enabled":true}))
            .send().await.unwrap().error_for_status().unwrap();
    }
    let after=source_next(&client,&base,"win","windows").await;
    assert_eq!(before["source_generation"],after["source_generation"]);
    let id=uuid::Uuid::new_v4().to_string();
    let sent:Value=headers(client.post(format!("{base}/source/desktop-request")),"win","windows")
        .json(&json!({"id":id,"path":"/snapshot","body":null})).send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
    assert_eq!(sent["state"],"pending");
    let denied=client.post(format!("{base}/source/desktop-next")).json(&json!({})).send().await.unwrap();
    assert_eq!(denied.status(),401);
    let jobs:Value=headers(client.post(format!("{base}/source/desktop-next")),"mac","macos")
        .json(&json!({})).send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
    assert_eq!(jobs["jobs"][0]["id"],id);
    headers(client.post(format!("{base}/source/desktop-authorize")),"mac","macos")
        .json(&json!({"id":id})).send().await.unwrap().error_for_status().unwrap();
    headers(client.post(format!("{base}/source/desktop-respond")),"mac","macos")
        .json(&json!({"id":id,"response":{"status":200,"body":{"requests":[]}}}))
        .send().await.unwrap().error_for_status().unwrap();
    let result:Value=headers(client.post(format!("{base}/source/desktop-result")),"win","windows")
        .json(&json!({"id":id})).send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
    assert_eq!(result["state"],"completed");
    assert_eq!(result["response"]["body"]["requests"],json!([]));
}

#[tokio::test]
async fn desktop_pairing_over_real_http_and_android_hello_preserves_source_session() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("hub.sqlite");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let _running = launch(&db, port);
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();
    ready(&client, &base).await;
    provision(&client, &base, "limited", "windows").await;
    let denied = headers(
        client.post(format!("{base}/source/pairing")),
        "limited",
        "windows",
    )
    .json(&json!({}))
    .send()
    .await
    .unwrap();
    assert_eq!(denied.status(), 403);
    client.post(format!("{base}/admin/provision")).bearer_auth(ADMIN).json(&json!({"kind":"source","credential":{"device_id":"win","client_kind":"windows","token_hash":hash(secret("win").as_bytes()),"scopes":["pairing.issue"],"windows_source":"win","mirror_sources":[],"file_browser_roots":[],"revoked_at":null}})).send().await.unwrap().error_for_status().unwrap();
    headers(
        client.post(format!("{base}/source/heartbeat")),
        "win",
        "windows",
    )
    .json(&json!({}))
    .send()
    .await
    .unwrap()
    .error_for_status()
    .unwrap();
    let qr: Value = headers(
        client.post(format!("{base}/source/pairing")),
        "win",
        "windows",
    )
    .header("X-Iterate-Source-Session", "desktop-short-session")
    .json(&json!({}))
    .send()
    .await
    .unwrap()
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    let sid = qr["pairing_session_id"].clone();
    assert_eq!(qr["version"], 2);
    assert_eq!(
        client
            .post(format!("{base}/source/pairing"))
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let claimed:Value=client.post(format!("{base}/api/android/pairing/claim")).json(&json!({"client_kind":"android","device_id":"new-phone","device_name":"APK contract","pairing_token":qr["pairing_token"]})).send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
    let mut request = format!("ws://127.0.0.1:{port}/ws")
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", claimed["device_token"].as_str().unwrap())
            .parse()
            .unwrap(),
    );
    request
        .headers_mut()
        .insert("X-Iterate-Device-Id", "new-phone".parse().unwrap());
    request
        .headers_mut()
        .insert("X-Iterate-Client-Kind", "android".parse().unwrap());
    let (mut phone, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    phone.send(Message::Text(json!({"message_type":"client_hello","payload":{"device_id":"new-phone","client_kind":"android","selected_transport_mode":"public_tunnel"}}).to_string())).await.unwrap();
    let mut snapshot = Value::Null;
    for _ in 0..40 {
        snapshot = headers(
            client.post(format!("{base}/source/pairing-session")),
            "win",
            "windows",
        )
        .json(&json!({"session_id":sid}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
        if snapshot["session"]["connected_at"].is_string() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(snapshot["session"]["connected_at"].is_string());
    assert_eq!(snapshot["session"]["state"], "claimed");
    headers(
        client.post(format!("{base}/source/pairing-applied")),
        "win",
        "windows",
    )
    .json(&json!({"session_id":sid,"token_hash":snapshot["credential"]["token_hash"]}))
    .send()
    .await
    .unwrap()
    .error_for_status()
    .unwrap();
    let snapshot: Value = headers(
        client.post(format!("{base}/source/pairing-session")),
        "win",
        "windows",
    )
    .json(&json!({"session_id":sid}))
    .send()
    .await
    .unwrap()
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(snapshot["session"]["state"], "connected");
    let sql = rusqlite::Connection::open(&db).unwrap();
    let live: String = sql
        .query_row(
            "SELECT value FROM objects WHERE kind='live' AND id='win'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let live: Value = serde_json::from_str(&live).unwrap();
    assert_eq!(live["session"], "test-source-session-win");
    assert_eq!(live["generation"], 1);
    phone.close(None).await.unwrap();
}

fn secret(id: &str) -> String {
    format!("temporary-process-device-{id}-0123456789")
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn launch(db: &std::path::Path, port: u16) -> Running {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_iterate-hub"));
    cmd.args([
        "--db",
        db.to_str().unwrap(),
        "--listen",
        &format!("127.0.0.1:{port}"),
    ]);
    cmd.env("ITERATE_HUB_ADMIN_HASH", hash(ADMIN.as_bytes()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    Running(cmd.spawn().expect("start actual hub binary"))
}
async fn ready(client: &reqwest::Client, base: &str) {
    for _ in 0..100 {
        if client.get(format!("{base}/health")).send().await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("hub failed to become ready")
}
fn headers(request: reqwest::RequestBuilder, id: &str, kind: &str) -> reqwest::RequestBuilder {
    request
        .bearer_auth(secret(id))
        .header("X-Iterate-Device-Id", id)
        .header("X-Iterate-Client-Kind", kind)
        .header(
            "X-Iterate-Source-Session",
            format!("test-source-session-{id}"),
        )
}
async fn provision(client: &reqwest::Client, base: &str, id: &str, kind: &str) {
    let role = if matches!(kind, "windows" | "macos") {
        "source"
    } else {
        "phone"
    };
    let response=client.post(format!("{base}/admin/provision")).bearer_auth(ADMIN).json(&json!({"kind":role,"credential":{"device_id":id,"client_kind":kind,"token_hash":hash(secret(id).as_bytes()),"scopes":["session.read","session.respond","window.show"],"windows_source":"win","mirror_sources":["mac"],"file_browser_roots":["C:\\Allowed"],"revoked_at":null}})).send().await.unwrap();
    assert!(
        response.status().is_success(),
        "{}",
        response.text().await.unwrap()
    );
}
async fn socket(
    base: &str,
    id: &str,
    kind: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let mut request = format!("{}/ws", base.replace("http://", "ws://"))
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", secret(id)).parse().unwrap(),
    );
    request
        .headers_mut()
        .insert("X-Iterate-Device-Id", id.parse().unwrap());
    request
        .headers_mut()
        .insert("X-Iterate-Client-Kind", kind.parse().unwrap());
    tokio_tungstenite::connect_async(request).await.unwrap().0
}
async fn next_type(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    ty: &str,
) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let message = socket.next().await.unwrap().unwrap();
            if let Message::Text(text) = message {
                let v: Value = serde_json::from_str(&text).unwrap();
                if v["message_type"] == ty {
                    return v;
                }
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn actual_process_http_ws_restart_reconcile_auth_and_live_revocation() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("hub.sqlite");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let process = launch(&db, port);
    ready(&client, &base).await;
    for (id, kind) in [("win", "windows"), ("mac", "macos"), ("phone", "android")] {
        provision(&client, &base, id, kind).await;
    }
    assert_eq!(
        headers(client.get(format!("{base}/admin/status")), "mac", "macos")
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        headers(
            client.get(format!("{base}/api/android/pairing/status")),
            "phone",
            "android"
        )
        .send()
        .await
        .unwrap()
        .status(),
        200
    );
    let route = format!("cross-mirror:{}", "a".repeat(64));
    let publication = json!({"source_request_id":"original-mac-request","version":1,"registration_key":"original-registration-key","route_id":route,"kind":"mirror","project_path":"","epoch":1,"published":true,"wire":{"message_type":"cross_device_mirror_state","payload":{"source":"cross_device_mirror","mirror_id":route,"request_id":route,"project_path":"","request":{"id":route,"project_path":"","message":"real process fixture","predefined_options":["yes"]}}}});
    assert!(headers(
        client.post(format!("{base}/source/publish")),
        "mac",
        "macos"
    )
    .json(&publication)
    .send()
    .await
    .unwrap()
    .status()
    .is_success());
    let mut phone = socket(&base, "phone", "android").await;
    let snapshot = next_type(&mut phone, "cross_device_mirror_state").await;
    assert_eq!(snapshot["payload"]["origin_device_id"], "mac");
    let wire = json!({"message_type":"cross_device_mirror_action","payload":{"mirror_id":route,"request_id":route,"client_action_id":"original-action","action":"submit","user_input":"unchanged reply","selected_options":["yes"]}});
    phone.send(Message::Text(wire.to_string())).await.unwrap();
    let pending = next_type(&mut phone, "cross_device_mirror_action_result").await;
    assert_eq!(pending["payload"]["status"], "pending");
    let batch: Value = headers(client.get(format!("{base}/source/next")), "mac", "macos")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(batch["jobs"].as_array().unwrap().len(), 1);
    let job = batch["jobs"][0].clone();
    assert_eq!(job["payload"]["wire"], wire);
    // Kill only the child started by this test; preserve the actual SQLite file.
    drop(phone);
    drop(process);
    let process = launch(&db, port);
    ready(&client, &base).await;
    let batch: Value = headers(client.get(format!("{base}/source/next")), "mac", "macos")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(batch["jobs"].as_array().unwrap().is_empty());
    assert_eq!(batch["reconcile"][0]["query_only"], true);
    let mut phone = socket(&base, "phone", "android").await;
    phone.send(Message::Text(wire.to_string())).await.unwrap();
    assert_eq!(
        next_type(&mut phone, "cross_device_mirror_action_result").await["payload"]["status"],
        "pending"
    );
    let ack = json!({"message_type":"cross_device_mirror_action_result","payload":{"mirror_id":route,"request_id":route,"source":"cross_device_mirror","client_action_id":"original-action","action":"submit","status":"accepted","result_kind":"mac_accepted","reason":null,"source_consumed":false}});
    let confirmation = json!({"action_id":job["payload"]["action_id"],"source_request_id":"original-mac-request","version":1,"registration_key":"original-registration-key","epoch":1,"state":"source_accepted","durable":true,"winner_record":"source-durable-winner","original_response":{"user_input":"unchanged reply","selected_options":["yes"]},"receipt":ack});
    assert!(!headers(
        client.post(format!("{base}/source/confirm")),
        "win",
        "windows"
    )
    .json(&confirmation)
    .send()
    .await
    .unwrap()
    .status()
    .is_success());
    let response = headers(
        client.post(format!("{base}/source/confirm")),
        "mac",
        "macos",
    )
    .json(&confirmation)
    .send()
    .await
    .unwrap();
    assert!(
        response.status().is_success(),
        "{}",
        response.text().await.unwrap()
    );
    // A queued snapshot can precede the terminal receipt on a new connection.
    let mut accepted = None;
    for _ in 0..4 {
        let result = next_type(&mut phone, "cross_device_mirror_action_result").await;
        if result["payload"]["status"] == "accepted" {
            accepted = Some(result);
            break;
        }
    }
    assert_eq!(accepted.unwrap(), ack);
    let revoke = client
        .post(format!("{base}/admin/revoke"))
        .bearer_auth(ADMIN)
        .json(&json!({"kind":"phone","device_id":"phone"}))
        .send()
        .await
        .unwrap();
    assert!(revoke.status().is_success());
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match phone.next().await {
                None | Some(Ok(Message::Close(_))) | Some(Err(_)) => break,
                _ => {}
            }
        }
    })
    .await
    .expect("revocation closes existing websocket");
    assert_eq!(
        headers(
            client.get(format!("{base}/api/android/pairing/status")),
            "phone",
            "android"
        )
        .send()
        .await
        .unwrap()
        .status(),
        401
    );
    drop(process);
}

async fn regression_fixture() -> (tempfile::TempDir, Running, reqwest::Client, String) {
    let dir = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let process = launch(&dir.path().join("hub.sqlite"), port);
    ready(&client, &base).await;
    for (id, kind) in [("win", "windows"), ("mac", "macos"), ("phone", "android")] {
        provision(&client, &base, id, kind).await;
    }
    (dir, process, client, base)
}
async fn control(client: &reqwest::Client, base: &str, value: Value) -> reqwest::Response {
    client
        .post(format!("{base}/admin/control"))
        .bearer_auth(ADMIN)
        .json(&value)
        .send()
        .await
        .unwrap()
}
async fn source_next(client: &reqwest::Client, base: &str, id: &str, kind: &str) -> Value {
    headers(client.get(format!("{base}/source/next")), id, kind)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap()
}
async fn acknowledge_barrier(client: &reqwest::Client, base: &str, id: &str, kind: &str) -> Value {
    let state = source_next(client, base, id, kind).await;
    let proof = json!({"epoch":state["epoch"],"freeze_generation":state["freeze_generation"],"source_generation":state["source_generation"],"legacy_connections_closed":true,"all_submit_paths_paused":true});
    headers(client.post(format!("{base}/source/barrier")), id, kind)
        .json(&proof)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    proof
}

#[tokio::test]
async fn regression_freeze_round_and_source_session_require_fresh_http_barriers() {
    let (_dir, _process, client, base) = regression_fixture().await;
    let first: Value = control(&client, &base, json!({"frozen":true}))
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let old_win = acknowledge_barrier(&client, &base, "win", "windows").await;
    acknowledge_barrier(&client, &base, "mac", "macos").await;
    control(&client, &base, json!({"frozen":false}))
        .await
        .error_for_status()
        .unwrap();
    let second: Value = control(&client, &base, json!({"frozen":true}))
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(second["epoch"], first["epoch"]);
    assert!(second["freeze_generation"].as_u64() > first["freeze_generation"].as_u64());
    assert!(!control(&client, &base, json!({"advance_epoch":true}))
        .await
        .status()
        .is_success());
    assert!(!headers(
        client.post(format!("{base}/source/barrier")),
        "win",
        "windows"
    )
    .json(&old_win)
    .send()
    .await
    .unwrap()
    .status()
    .is_success());
    acknowledge_barrier(&client, &base, "win", "windows").await;
    acknowledge_barrier(&client, &base, "mac", "macos").await;
    // Draining resumes a submit path, so returning to hard freeze needs another
    // barrier round even though neither route epoch nor device IDs changed.
    control(&client, &base, json!({"drain":true}))
        .await
        .error_for_status()
        .unwrap();
    control(&client, &base, json!({"drain":false}))
        .await
        .error_for_status()
        .unwrap();
    assert!(!control(&client, &base, json!({"advance_epoch":true}))
        .await
        .status()
        .is_success());
    let previous_win = acknowledge_barrier(&client, &base, "win", "windows").await;
    acknowledge_barrier(&client, &base, "mac", "macos").await;
    let source_request = |request: reqwest::RequestBuilder| {
        request
            .bearer_auth(secret("win"))
            .header("X-Iterate-Device-Id", "win")
            .header("X-Iterate-Client-Kind", "windows")
            .header("X-Iterate-Source-Session", "restarted-source-process")
    };
    source_request(client.post(format!("{base}/source/heartbeat")))
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    assert!(!control(&client, &base, json!({"advance_epoch":true}))
        .await
        .status()
        .is_success());
    assert!(
        !source_request(client.post(format!("{base}/source/barrier")))
            .json(&previous_win)
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    let state: Value = source_request(client.get(format!("{base}/source/next")))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(state["source_generation"].as_u64() > previous_win["source_generation"].as_u64());
    let proof = json!({"epoch":state["epoch"],"freeze_generation":state["freeze_generation"],"source_generation":state["source_generation"],"legacy_connections_closed":true,"all_submit_paths_paused":true});
    source_request(client.post(format!("{base}/source/barrier")))
        .json(&proof)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let advanced: Value = control(&client, &base, json!({"advance_epoch":true}))
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(advanced["epoch"], 2);
    println!("P1 freeze: thaw/refreeze, drain round, old proof replay and changed source session blocked; only fresh barriers advanced epoch");
}

async fn publish_windows_fixture(client: &reqwest::Client, base: &str) {
    let publication = json!({"source_request_id":"win-original","version":1,"registration_key":"win-original-registration","route_id":"win-request","kind":"mcp","project_path":"C:/Allowed","epoch":1,"published":true,"wire":{"message_type":"mcp_state","payload":{"timeline_route_id":"timeline-one","request":{"id":"win-request","project_path":"C:/Allowed","message":"review regression fixture"}}}});
    headers(
        client.post(format!("{base}/source/publish")),
        "win",
        "windows",
    )
    .json(&publication)
    .send()
    .await
    .unwrap()
    .error_for_status()
    .unwrap();
}

#[tokio::test]
async fn regression_offline_windows_timeline_keeps_mac_socket_and_denials() {
    let (dir, _process, client, base) = regression_fixture().await;
    publish_windows_fixture(&client, &base).await;
    let route = format!("cross-mirror:{}", "b".repeat(64));
    let mut publication = json!({"source_request_id":"offline-mac-original","version":1,"registration_key":"offline-mac-registration","route_id":route,"kind":"mirror","project_path":"","epoch":1,"published":true,"wire":{"message_type":"cross_device_mirror_state","payload":{"source":"cross_device_mirror","mirror_id":route,"request_id":route,"project_path":"","request":{"id":route,"project_path":"","message":"before Windows offline"}}}});
    headers(client.post(format!("{base}/source/publish")), "mac", "macos")
        .json(&publication).send().await.unwrap().error_for_status().unwrap();
    let mut phone = socket(&base, "phone", "android").await;
    next_type(&mut phone, "cross_device_mirror_state").await;

    // Expire only the test's Windows lease; preserve source session and Hub boot.
    let sql = rusqlite::Connection::open(dir.path().join("hub.sqlite")).unwrap();
    let live: String = sql.query_row("SELECT value FROM objects WHERE kind='live' AND id='win'", [], |r| r.get(0)).unwrap();
    let mut live: Value = serde_json::from_str(&live).unwrap();
    live["at"] = json!(chrono::Utc::now().timestamp() - 60);
    let encoded = live.to_string();
    sql.execute("UPDATE objects SET value=?1,charge=?2 WHERE kind='live' AND id='win'", rusqlite::params![encoded, encoded.len() as i64]).unwrap();

    for request_id in ["win-request", "timeline-one"] {
        phone.send(Message::Text(json!({"message_type":"request_timeline_sync","payload":{"request_id":request_id,"project_path":"C:/Allowed"}}).to_string())).await.unwrap();
        let status = next_type(&mut phone, "hub_status").await;
        assert_eq!(status["payload"], json!({"windows_available":false,"reason":"windows_offline"}));
    }
    publication["version"] = json!(2);
    let new_route = format!("cross-mirror:{}", "c".repeat(64));
    publication["route_id"] = json!(new_route);
    publication["wire"]["payload"]["mirror_id"] = json!(new_route);
    publication["wire"]["payload"]["request_id"] = json!(new_route);
    publication["wire"]["payload"]["request"]["id"] = json!(new_route);
    publication["wire"]["payload"]["request"]["message"] = json!("Mac after offline timeline");
    headers(client.post(format!("{base}/source/publish")), "mac", "macos")
        .json(&publication).send().await.unwrap().error_for_status().unwrap();
    let mirror = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = next_type(&mut phone, "cross_device_mirror_state").await;
            if event["payload"]["request"]["id"] == new_route {
                break event;
            }
        }
    }).await.expect("same socket must receive new Mac route");
    assert_eq!(mirror["payload"]["origin_device_id"], "mac");
    assert_eq!(mirror["payload"]["request"]["message"], "Mac after offline timeline");
    let jobs: i64 = sql.query_row("SELECT count(*) FROM objects WHERE kind='job'", [], |r| r.get(0)).unwrap();
    assert_eq!(jobs, 0, "offline timeline must not queue work");

    // Errors other than windows_offline must retain the prior fail-closed policy.
    for (request_id, project_path) in [("win-request", "C:/Denied"), ("missing-route", "C:/Allowed"), (new_route.as_str(), "")] {
        let mut denied = socket(&base, "phone", "android").await;
        next_type(&mut denied, "cross_device_mirror_state").await;
        denied.send(Message::Text(json!({"message_type":"request_timeline_sync","payload":{"request_id":request_id,"project_path":project_path}}).to_string())).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match denied.next().await {
                    None | Some(Ok(Message::Close(_))) | Some(Err(_)) => break,
                    Some(Ok(Message::Text(text))) => assert_ne!(serde_json::from_str::<Value>(&text).unwrap()["message_type"], "hub_status"),
                    _ => {}
                }
            }
        }).await.expect("denied timeline must close socket");
    }
    client.post(format!("{base}/admin/revoke")).bearer_auth(ADMIN)
        .json(&json!({"kind":"phone","device_id":"phone"})).send().await.unwrap().error_for_status().unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match phone.next().await {
                None | Some(Ok(Message::Close(_))) | Some(Err(_)) => break,
                _ => {}
            }
        }
    }).await.expect("revoked phone must still close socket");
    println!("Offline timeline: both authorized routes return status; same WS receives fresh Mac mirror; zero jobs; wrong project, unknown route, Mac history and revoked phone remain closed");
}
async fn wait_job(client: &reqwest::Client, base: &str) -> Value {
    for _ in 0..50 {
        let batch = source_next(client, base, "win", "windows").await;
        if let Some(j) = batch["jobs"].as_array().unwrap().first() {
            return j.clone();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("expected source job")
}

#[tokio::test]
async fn regression_timeline_with_sync_reason_reaches_only_requesting_phone_ws() {
    let (_dir, _process, client, base) = regression_fixture().await;
    provision(&client, &base, "other-phone", "android").await;
    publish_windows_fixture(&client, &base).await;
    let mut requester = socket(&base, "phone", "android").await;
    let mut same_phone_other_ws = socket(&base, "phone", "android").await;
    let mut other_phone = socket(&base, "other-phone", "android").await;
    for s in [&mut requester, &mut same_phone_other_ws, &mut other_phone] {
        next_type(s, "mcp_state").await;
    }
    requester.send(Message::Text(json!({"message_type":"request_sync","payload":{"sync_reason":"android_connect:regression"}}).to_string())).await.unwrap();
    let sync = wait_job(&client, &base).await;
    let templates = json!({"message_type":"mcp_state","payload":{"sync_response":true,"sync_reason":"android_connect:regression","customPrompts":{"enabled":true,"prompts":[]}}});
    headers(
        client.post(format!("{base}/source/respond")),
        "win",
        "windows",
    )
    .json(&json!({"job_id":sync["id"],"epoch":1,"response":{"events":[templates]}}))
    .send()
    .await
    .unwrap()
    .error_for_status()
    .unwrap();
    loop {
        let event = next_type(&mut requester, "mcp_state").await;
        if event["payload"]["sync_response"] == true {
            break;
        }
    }
    requester.send(Message::Text(json!({"message_type":"request_timeline_sync","payload":{"request_id":"timeline-one","project_path":"C:/Allowed"}}).to_string())).await.unwrap();
    let timeline = wait_job(&client, &base).await;
    assert_eq!(timeline["payload"]["operation"], "timeline");
    assert!(timeline["payload"]["wire"]["payload"]
        .get("sync_reason")
        .is_none());
    let event = json!({"message_type":"timeline_sync_snapshot","payload":{"timeline_route_id":"timeline-one","project_path":"C:/Allowed","conversation_id":"conversation-one","timelineNodes":[{"id":"node1","node_type":"assistant","content":"source timeline result"}]}});
    let response = json!({"job_id":timeline["id"],"epoch":1,"response":{"events":[event]}});
    assert!(!headers(
        client.post(format!("{base}/source/respond")),
        "mac",
        "macos"
    )
    .json(&response)
    .send()
    .await
    .unwrap()
    .status()
    .is_success());
    headers(
        client.post(format!("{base}/source/respond")),
        "win",
        "windows",
    )
    .json(&response)
    .send()
    .await
    .unwrap()
    .error_for_status()
    .unwrap();
    assert_eq!(
        next_type(&mut requester, "timeline_sync_snapshot").await,
        event
    );
    assert!(tokio::time::timeout(
        Duration::from_millis(1200),
        next_type(&mut same_phone_other_ws, "timeline_sync_snapshot")
    )
    .await
    .is_err());
    assert!(tokio::time::timeout(
        Duration::from_millis(1200),
        next_type(&mut other_phone, "timeline_sync_snapshot")
    )
    .await
    .is_err());
    drop(requester);
    let mut reconnected = socket(&base, "phone", "android").await;
    assert!(tokio::time::timeout(
        Duration::from_millis(1200),
        next_type(&mut reconnected, "timeline_sync_snapshot")
    )
    .await
    .is_err());
    println!("P1 timeline: nonempty sync_reason plus legacy timeline without reason delivered; wrong source and other WS/phone/reconnected session excluded");
}

#[tokio::test]
async fn regression_same_ws_restores_identical_active_snapshot_after_real_source_timeout() {
    let (_dir, _process, client, base) = regression_fixture().await;
    publish_windows_fixture(&client, &base).await;
    let mut phone = socket(&base, "phone", "android").await;
    let active = next_type(&mut phone, "mcp_state").await;
    assert!(active["payload"]["request"].is_object());
    let started = std::time::Instant::now();
    tokio::time::timeout(Duration::from_secs(52), async {
        loop {
            let message = phone.next().await.unwrap().unwrap();
            if let Message::Text(text) = message {
                let v: Value = serde_json::from_str(&text).unwrap();
                if v["message_type"] == "mcp_state"
                    && v["payload"]["request_id"] == "win-request"
                    && v["payload"]["request"].is_null()
                {
                    assert_eq!(v["payload"]["showMcpPopup"], false);
                    break;
                }
            }
        }
    })
    .await
    .expect("real 45-second source heartbeat expiry closes Windows snapshot");
    assert!(started.elapsed() >= Duration::from_secs(44));
    headers(
        client.post(format!("{base}/source/heartbeat")),
        "win",
        "windows",
    )
    .json(&json!({}))
    .send()
    .await
    .unwrap()
    .error_for_status()
    .unwrap();
    let restored = next_type(&mut phone, "mcp_state").await;
    assert_eq!(restored, active);
    assert!(tokio::time::timeout(
        Duration::from_millis(1200),
        next_type(&mut phone, "mcp_state")
    )
    .await
    .is_err());
    let status: Value = client
        .get(format!("{base}/admin/status"))
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(status["actions"].as_array().unwrap().is_empty());
    println!("P2 snapshot: same WS active -> closed after real expiry -> identical active restored; unchanged state not repeated and no actions created ({} seconds)",started.elapsed().as_secs());
}
