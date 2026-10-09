use base64::{engine::general_purpose::STANDARD, Engine};
use iterate_hub::{protocol::*, store::Store};
use serde_json::{json, Value};
use tempfile::TempDir;

const SECRET: &str = "temporary-component-credential-0123456789";

fn announce_desktops(db: &mut Store) {
    db.desktop_announce(&credential("win","windows"),&json!({"own_hash":hash(b"win-pair-token"),
        "peer_hash":hash(b"mac-pair-token"),"name":"Windows","enabled":true})).unwrap();
    db.desktop_announce(&credential("mac","macos"),&json!({"own_hash":hash(b"mac-pair-token"),
        "peer_hash":hash(b"win-pair-token"),"name":"Mac","enabled":true})).unwrap();
}

#[test]
fn desktop_public_relay_requires_mutual_pairing_and_actual_response() {
    let (_dir,mut db,_)=setup(16*1024*1024);
    db.desktop_announce(&credential("win","windows"),&json!({"own_hash":hash(b"win-pair-token"),
        "peer_hash":hash(b"mac-pair-token"),"name":"Windows","enabled":true})).unwrap();
    assert_eq!(db.desktop_status("win").unwrap()["connected"],false);
    let id=uuid::Uuid::new_v4().to_string();
    let request=json!({"id":id,"path":"/snapshot","body":null});
    assert!(db.desktop_request("win",request.clone()).is_err());
    announce_desktops(&mut db);
    assert_eq!(db.desktop_status("win").unwrap()["peer_name"],"Mac");
    assert_eq!(db.desktop_request("win",request.clone()).unwrap()["state"],"pending");
    assert_eq!(db.desktop_request("win",request).unwrap()["state"],"pending");
    assert_eq!(db.desktop_next("mac").unwrap()["jobs"].as_array().unwrap().len(),1);
    assert!(db.desktop_next("win").unwrap()["jobs"].as_array().unwrap().is_empty());
    assert!(db.desktop_result("mac",&id).is_err());
    let response=json!({"id":id,"response":{"status":200,"body":{"requests":[]}}});
    assert!(db.desktop_respond("win",response.clone()).is_err());
    db.desktop_respond("mac",response).unwrap();
    assert_eq!(db.desktop_result("win",&id).unwrap()["response"]["body"]["requests"],json!([]));
    assert!(db.desktop_respond("mac",json!({"id":id,"response":{"status":200,"body":{"different":true}}})).is_err());
    assert!(db.desktop_request("win",json!({"id":id,"path":"/submit","body":{}})).is_err());
}

#[test]
fn desktop_public_relay_survives_restart_and_rejects_other_routes() {
    let (dir,mut db,_)=setup(16*1024*1024);
    announce_desktops(&mut db);
    let id=uuid::Uuid::new_v4().to_string();
    db.desktop_request("mac",json!({"id":id,"path":"/submit","body":{"key":"original","response":{"text":"reply"}}})).unwrap();
    drop(db);
    let mut db=Store::open(&dir.path().join("hub.sqlite"),16*1024*1024).unwrap();
    announce_desktops(&mut db);
    assert_eq!(db.desktop_result("mac",&id).unwrap()["state"],"pending");
    let next=db.desktop_next("win").unwrap();
    assert_eq!(next["jobs"][0]["body"]["response"]["text"],"reply");
    assert!(db.desktop_request("win",json!({"id":uuid::Uuid::new_v4().to_string(),"path":"/admin/control","body":{}})).is_err());
    db.control(json!({"frozen":true,"drain":false})).unwrap();
    assert!(db.desktop_next("win").unwrap()["jobs"].as_array().unwrap().is_empty());
    assert!(db.desktop_request("win",json!({"id":uuid::Uuid::new_v4().to_string(),"path":"/snapshot","body":null})).is_err());
}

#[test]
fn desktop_public_relay_rechecks_pairing_after_jobs_are_queued() {
    let (_dir,mut db,_)=setup(16*1024*1024);
    announce_desktops(&mut db);
    let id=uuid::Uuid::new_v4().to_string();
    db.desktop_request("win",json!({"id":id,"path":"/snapshot","body":null})).unwrap();
    db.desktop_announce(&credential("mac","macos"),&json!({"own_hash":hash(b"new-mac-token"),
        "peer_hash":hash(b"win-pair-token"),"name":"Mac","enabled":true})).unwrap();
    assert_eq!(db.desktop_status("win").unwrap()["connected"],false);
    assert!(db.desktop_next("mac").is_err());
    assert!(db.desktop_result("win",&id).is_err());
    announce_desktops(&mut db);
    db.revoke("source","mac").unwrap();
    assert_eq!(db.desktop_status("win").unwrap()["connected"],false);
}
fn credential(id: &str, kind: &str) -> Credential {
    Credential {
        device_id: id.into(),
        client_kind: kind.into(),
        token_hash: hash(format!("{SECRET}-{id}").as_bytes()),
        scopes: vec![
            "session.read".into(),
            "session.respond".into(),
            "window.show".into(),
        ],
        revoked_at: None,
        file_browser_roots: vec!["C:\\Allowed".into()],
        windows_source: "win".into(),
        mirror_sources: vec!["mac".into()],
    }
}
fn setup(budget: u64) -> (TempDir, Store, Credential) {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Store::open(&dir.path().join("hub.sqlite"), budget).unwrap();
    db.provision("source", credential("win", "windows"))
        .unwrap();
    db.provision("source", credential("mac", "macos")).unwrap();
    let phone = credential("phone", "android");
    db.provision("phone", phone.clone()).unwrap();
    (dir, db, phone)
}
fn publication(kind: &str, version: u64) -> Publication {
    let route = if kind == "mirror" {
        format!("cross-mirror:{:064x}", version)
    } else if kind == "native" {
        format!("native-codex:req{version}")
    } else {
        format!("win-request-{version}")
    };
    let project = if kind == "mirror" { "" } else { "C:\\Allowed" };
    let payload = if kind == "mirror" {
        json!({"mirror_id":route,"request_id":route,"source":"cross_device_mirror","project_path":"","request":{"id":route,"message":"published Mac text","predefined_options":["yes","no"],"project_path":""}})
    } else {
        json!({"source":if kind=="native"{"codex_native"}else{"mcp"},"timeline_route_id":if kind=="native"{"native-cli:history1"}else{"timeline1"},"request":{"id":route,"project_path":project,"message":"published Windows text","predefined_options":["yes"]}})
    };
    Publication {
        source_request_id: format!("original-{kind}"),
        version,
        registration_key: format!("registration-{kind}"),
        route_id: route,
        kind: kind.into(),
        project_path: project.into(),
        epoch: 1,
        published: true,
        wire: json!({"message_type":if kind=="mirror"{"cross_device_mirror_state"}else{"mcp_state"},"payload":payload}),
    }
}
fn action(p: &Publication, id: &str) -> Value {
    let mut payload = json!({"request_id":p.route_id,"project_path":p.project_path,"client_action_id":id,"action":"submit","user_input":"hello","selected_options":["yes"]});
    if p.kind == "mirror" {
        payload["mirror_id"] = json!(p.route_id);
    }
    json!({"message_type":if p.kind=="mirror"{"cross_device_mirror_action"}else{"mcp_action"},"payload":payload})
}
fn confirmation(p: &Publication, job: &Value, id: &str) -> Confirmation {
    let mut payload = json!({"request_id":p.route_id,"project_path":p.project_path,"client_action_id":id,"action":"submit","status":"delivered","delivered":true});
    if p.kind == "mirror" {
        payload["mirror_id"] = json!(p.route_id);
        payload["source"] = json!("cross_device_mirror");
        payload["status"] = json!("accepted");
        payload["result_kind"] = json!("mac_accepted");
    }
    if p.kind == "native" {
        payload["source"] = json!("codex_native");
        payload["final_result"] = json!(true);
        payload["result_kind"] = json!("cli_received");
    }
    Confirmation {
        action_id: job["payload"]["action_id"].as_str().unwrap().into(),
        source_request_id: p.source_request_id.clone(),
        version: p.version,
        registration_key: p.registration_key.clone(),
        epoch: 1,
        state: "source_accepted".into(),
        durable: true,
        winner_record: Some("original-source-winner-1".into()),
        receipt: Some(
            json!({"message_type":if p.kind=="mirror"{"cross_device_mirror_action_result"}else{"mcp_action_result"},"payload":payload}),
        ),
        original_response: Some(json!({"user_input":"hello","selected_options":["yes"]})),
        definitely_not_executed: false,
    }
}

#[test]
fn byte_bounded_dispatch_and_legacy_unknown_pages_survive_restart() {
    let (dir, mut db, phone) = setup(MAX_BUDGET);
    let p = publication("mcp", 1);
    db.publish("win", p.clone()).unwrap();
    for i in 0..16 {
        let mut wire = action(&p, &format!("large-{i}"));
        wire["payload"]["user_input"] = json!("x".repeat(180_000));
        db.action(&phone, wire).unwrap();
    }
    let first = db.next("win").unwrap();
    assert!(serde_json::to_vec(&first).unwrap().len() <= 2 * 1024 * 1024);
    assert!(!first["jobs"].as_array().unwrap().is_empty());
    assert!(first["jobs"].as_array().unwrap().len() < 16);
    drop(db);
    // Reproduce an older hub's already-dispatched oversized batch, not merely
    // a new batch produced by the fixed dispatcher.
    let sql = rusqlite::Connection::open(dir.path().join("hub.sqlite")).unwrap();
    sql.execute(
        "UPDATE objects SET value=json_set(value,'$.state','dispatched') WHERE kind='job'",
        [],
    )
    .unwrap();
    sql.execute("UPDATE objects SET value=json_set(value,'$.state','confirmation_unknown') WHERE kind='action'",[]).unwrap();
    drop(sql);
    let mut seen = std::collections::HashSet::new();
    for _ in 0..8 {
        let mut db = Store::open(&dir.path().join("hub.sqlite"), MAX_BUDGET).unwrap();
        let page = db.next("win").unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() <= 2 * 1024 * 1024);
        assert!(page["jobs"].as_array().unwrap().is_empty());
        for j in page["reconcile"].as_array().unwrap() {
            assert_eq!(j["query_only"], true);
            seen.insert(j["id"].as_str().unwrap().to_owned());
        }
    }
    assert_eq!(
        seen.len(),
        16,
        "unresolved first-page jobs must not starve later receipts"
    );
}

#[test]
fn ordinary_timeline_registration_and_legacy_mapping_recovery() {
    let (dir, mut db, phone) = setup(16 * 1024 * 1024);
    let mut p = publication("mcp", 1);
    p.wire["payload"]
        .as_object_mut()
        .unwrap()
        .remove("timeline_route_id");
    db.publish("win", p.clone()).unwrap();
    let id=db.rpc(&phone,"timeline",json!({"message_type":"request_timeline_sync","payload":{"request_id":p.route_id,"project_path":p.project_path}})).unwrap();
    assert_eq!(db.next("win").unwrap()["jobs"][0]["id"], id);
    // A persisted pre-fix publication has no timeline map. Its authenticated
    // original identity can repair the map without mutating the publication.
    let sql = rusqlite::Connection::open(dir.path().join("hub.sqlite")).unwrap();
    sql.execute("DELETE FROM objects WHERE kind='timeline'", [])
        .unwrap();
    drop(sql);
    let response = json!({"events":[{"message_type":"timeline_sync_snapshot","payload":{"request_id":p.route_id,"timeline_route_id":p.route_id,"project_path":p.project_path,"timelineNodes":[{"id":"original-node"}]}}]});
    db.rpc_response("win", json!({"job_id":id,"epoch":1,"response":response}))
        .unwrap();
    assert_eq!(db.rpc_result(&phone, &id).unwrap(), Some(response));
}

#[test]
fn durable_action_duplicate_conflict_and_restart_reconciliation() {
    let (dir, mut db, phone) = setup(16 * 1024 * 1024);
    let p = publication("mirror", 1);
    db.publish("mac", p.clone()).unwrap();
    db.publish("mac", p.clone()).unwrap();
    let wire = action(&p, "client1");
    let pending = db.action(&phone, wire.clone()).unwrap();
    assert_eq!(pending["payload"]["status"], "pending");
    assert_eq!(db.action(&phone, wire.clone()).unwrap(), pending);
    let mut conflict = wire.clone();
    conflict["payload"]["user_input"] = json!("different");
    assert!(db
        .action(&phone, conflict)
        .unwrap_err()
        .to_string()
        .contains("conflict"));
    let next = db.next("mac").unwrap();
    let dispatched = next["jobs"][0].clone();
    assert_eq!(dispatched["payload"]["wire"], wire);
    assert!(dispatched["payload"]["wire"]["payload"]
        .get("metadata")
        .is_none());
    drop(db);
    let mut db = Store::open(&dir.path().join("hub.sqlite"), 16 * 1024 * 1024).unwrap();
    let replay = db.next("mac").unwrap();
    assert_eq!(replay["jobs"].as_array().unwrap().len(), 0);
    assert_eq!(replay["reconcile"][0]["query_only"], true);
    assert_eq!(
        db.action(&phone, wire.clone()).unwrap()["payload"]["status"],
        "pending"
    );
    let accepted = db
        .confirm("mac", confirmation(&p, &dispatched, "client1"))
        .unwrap();
    assert_eq!(accepted["payload"]["status"], "accepted");
    assert_eq!(accepted["payload"]["result_kind"], "mac_accepted");
    let mut handoff = confirmation(&p, &dispatched, "client1");
    handoff.state = "response_handoff".into();
    assert_eq!(db.confirm("mac", handoff).unwrap(), accepted);
    drop(db);
    let mut db = Store::open(&dir.path().join("hub.sqlite"), 16 * 1024 * 1024).unwrap();
    assert_eq!(db.action(&phone, wire).unwrap(), accepted);
    assert_eq!(
        db.status().unwrap()["actions"][0]["state"],
        "response_handoff"
    );
}

#[test]
fn source_authority_wrong_source_version_and_winner_conflicts() {
    let (_dir, mut db, phone) = setup(16 * 1024 * 1024);
    let p = publication("mirror", 1);
    db.publish("mac", p.clone()).unwrap();
    db.action(&phone, action(&p, "a1")).unwrap();
    let j = db.next("mac").unwrap()["jobs"][0].clone();
    assert!(db.confirm("win", confirmation(&p, &j, "a1")).is_err());
    let mut c = confirmation(&p, &j, "a1");
    c.version = 2;
    assert!(db.confirm("mac", c).is_err());
    let mut c = confirmation(&p, &j, "a1");
    c.durable = false;
    assert!(db.confirm("mac", c).is_err());
    let mut c = confirmation(&p, &j, "a1");
    c.receipt.as_mut().unwrap()["payload"]["status"] = json!("queued");
    assert!(db.confirm("mac", c).is_err());
    db.action(&phone, action(&p, "a2")).unwrap();
    let j2 = db.next("mac").unwrap()["jobs"][0].clone();
    db.confirm("mac", confirmation(&p, &j, "a1")).unwrap();
    let mut c = confirmation(&p, &j2, "a2");
    c.winner_record = Some("other-winner".into());
    assert!(db
        .confirm("mac", c)
        .unwrap_err()
        .to_string()
        .contains("source_winner_conflict"));
}

#[test]
fn queued_never_means_accepted_and_not_executed_requires_proof() {
    let (_dir, mut db, phone) = setup(8 * 1024 * 1024);
    let p = publication("mirror", 1);
    db.publish("mac", p.clone()).unwrap();
    db.action(&phone, action(&p, "a")).unwrap();
    let j = db.next("mac").unwrap()["jobs"][0].clone();
    let mut c = confirmation(&p, &j, "a");
    c.state = "unknown".into();
    assert_eq!(
        db.confirm("mac", c).unwrap()["payload"]["status"],
        "pending"
    );
    let mut c = confirmation(&p, &j, "a");
    c.state = "not_executed".into();
    assert!(db.confirm("mac", c.clone()).is_err());
    c.definitely_not_executed = true;
    db.confirm("mac", c).unwrap();
    assert_eq!(db.next("mac").unwrap()["jobs"].as_array().unwrap().len(), 1);
}

#[test]
fn android_normal_native_and_close_mirror_receipts() {
    for kind in ["mcp", "native", "mirror"] {
        let (_dir, mut db, phone) = setup(8 * 1024 * 1024);
        let p = publication(kind, 1);
        let source = if kind == "mirror" { "mac" } else { "win" };
        db.publish(source, p.clone()).unwrap();
        let mut wire = action(&p, "a");
        if kind == "mirror" {
            wire["payload"]["action"] = json!("close_mirror");
        }
        let pending = db.action(&phone, wire.clone()).unwrap();
        assert_eq!(pending["payload"]["status"], "pending");
        let j = db.next(source).unwrap()["jobs"][0].clone();
        let mut c = confirmation(&p, &j, "a");
        if kind == "mirror" {
            let payload = &mut c.receipt.as_mut().unwrap()["payload"];
            payload["action"] = json!("close_mirror");
            payload["result_kind"] = json!("mirror_closed");
        }
        let ack = db.confirm(source, c).unwrap();
        assert_eq!(ack["payload"]["client_action_id"], "a");
        if kind == "native" {
            assert_eq!(ack["payload"]["final_result"], true);
            assert_eq!(ack["payload"]["result_kind"], "cli_received");
            let mut query = wire;
            query["payload"]["native_query"] = json!(true);
            assert_eq!(db.action(&phone, query).unwrap(), ack);
        } else if kind == "mirror" {
            assert_eq!(ack["payload"]["result_kind"], "mirror_closed");
        }
    }
}

#[test]
fn stale_versions_routes_and_offline_deferred_cannot_publish() {
    let (_dir, mut db, phone) = setup(8 * 1024 * 1024);
    let p = publication("mirror", 1);
    db.publish("mac", p.clone()).unwrap();
    let p2 = publication("mirror", 2);
    db.publish("mac", p2.clone()).unwrap();
    assert!(db.action(&phone, action(&p, "old")).is_err());
    let mut reuse = publication("mirror", 3);
    reuse.route_id = p2.route_id.clone();
    reuse.wire = p2.wire.clone();
    assert!(db.publish("mac", reuse).is_err());
    let mut deferred = publication("mirror", 3);
    deferred.published = false;
    assert!(db.publish("mac", deferred).is_err());
    let mut invalid = publication("mirror", 3);
    invalid.wire["payload"]["project_path"] = json!("/Users/private");
    assert!(db.publish("mac", invalid).is_err());
}

#[test]
fn credential_scope_revocation_and_ios_are_preserved() {
    let (_dir, mut db, phone) = setup(8 * 1024 * 1024);
    let p = publication("mirror", 1);
    db.publish("mac", p.clone()).unwrap();
    assert!(db
        .authenticate("phone", "phone", "android", &format!("{SECRET}-phone"))
        .is_ok());
    assert!(db
        .authenticate("source", "mac", "windows", &format!("{SECRET}-mac"))
        .is_err());
    let mut denied = phone.clone();
    denied.scopes = vec!["session.read".into()];
    assert!(db.action(&denied, action(&p, "a")).is_err());
    denied = phone.clone();
    denied.mirror_sources.clear();
    assert!(db.action(&denied, action(&p, "a")).is_err());
    let mut ios = credential("ios", "ios");
    ios.revoked_at = Some("2026-01-01T00:00:00Z".into());
    db.provision("phone", ios).unwrap();
    assert!(db
        .authenticate("phone", "ios", "ios", &format!("{SECRET}-ios"))
        .is_err());
    db.action(&phone, action(&p, "a")).unwrap();
    db.revoke("phone", "phone").unwrap();
    assert!(db
        .authenticate("phone", "phone", "android", &format!("{SECRET}-phone"))
        .is_err());
    assert!(db.next("mac").unwrap()["jobs"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(db.status().unwrap()["actions"][0]["state"], "rejected");
}

#[test]
fn pairing_once_persistent_and_hash_only() {
    let (dir, mut db, _) = setup(8 * 1024 * 1024);
    let pair = db.pairing(credential("unused", "android")).unwrap();
    let claim = json!({"client_kind":"android","device_id":"newphone","pairing_token":pair["pairing_token"]});
    let result = db.claim(claim.clone()).unwrap();
    assert!(field(&result, "device_token").starts_with("dt_"));
    assert!(db.claim(claim).is_err());
    drop(db);
    let db = Store::open(&dir.path().join("hub.sqlite"), 8 * 1024 * 1024).unwrap();
    assert!(db
        .authenticate(
            "phone",
            "newphone",
            "android",
            field(&result, "device_token")
        )
        .is_ok());
    let bytes = std::fs::read(dir.path().join("hub.sqlite")).unwrap();
    let content = String::from_utf8_lossy(&bytes);
    assert!(!content.contains(field(&result, "device_token")));
    assert!(!content.contains(field(&pair, "pairing_token")));
}

#[test]
fn desktop_pairing_preserves_active_grants_and_waits_for_local_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Store::open(&dir.path().join("hub.sqlite"), MAX_BUDGET).unwrap();
    let old = credential("phone", "android");
    let mut source = credential("win", "windows");
    assert!(db.source_pairing(&source).is_err());
    source.scopes.push("pairing.issue".into());
    db.provision("source", source.clone()).unwrap();
    db.provision("source", credential("mac", "macos")).unwrap();
    db.provision("phone", old.clone()).unwrap();
    let mut mac = credential("mac", "macos");
    mac.scopes.push("pairing.issue".into());
    assert!(db.source_pairing(&mac).is_err());
    let qr = db.source_pairing(&source).unwrap();
    let sid = field(&qr, "pairing_session_id");
    assert_eq!(qr["version"], 2);
    assert_eq!(qr["transport_mode"], "public_tunnel");
    assert_eq!(qr["candidates"][0]["base_url"], qr["base_url"]);
    assert_eq!(qr["candidates"][0]["health"], "healthy");
    assert_eq!(
        db.source_pairing_session(&source, sid).unwrap()["session"]["state"],
        "pending"
    );
    let body = json!({"client_kind":"android","device_id":"phone","device_name":"Existing Android","pairing_token":qr["pairing_token"]});
    let claim = db.claim(body.clone()).unwrap();
    assert!(db.claim(body).is_err());
    assert!(db
        .authenticate("phone", "phone", "android", &format!("{SECRET}-phone"))
        .is_err());
    let current = db
        .authenticate("phone", "phone", "android", field(&claim, "device_token"))
        .unwrap();
    assert_eq!(current.scopes, old.scopes);
    assert_eq!(current.file_browser_roots, old.file_browser_roots);
    assert_eq!(current.mirror_sources, old.mirror_sources);
    db.pairing_connected(&current, "public_tunnel").unwrap();
    assert_eq!(
        db.source_pairing_session(&source, sid).unwrap()["session"]["state"],
        "claimed"
    );
    assert!(db
        .source_pairing_applied(&source, &json!({"session_id":sid,"token_hash":"wrong"}))
        .is_err());
    db.source_pairing_applied(
        &source,
        &json!({"session_id":sid,"token_hash":current.token_hash}),
    )
    .unwrap();
    drop(db);
    let mut db = Store::open(&dir.path().join("hub.sqlite"), MAX_BUDGET).unwrap();
    assert_eq!(
        db.source_pairing_session(&source, sid).unwrap()["session"]["state"],
        "connected"
    );
    db.revoke("phone", "phone").unwrap();
    assert_eq!(
        db.source_pairing_session(&source, sid).unwrap()["session"]["state"],
        "failed"
    );
    assert!(db.source_pairing_session(&source, sid).unwrap()["credential"].is_null());
    let fresh = db.source_pairing(&source).unwrap();
    let claim=db.claim(json!({"client_kind":"android","device_id":"phone","pairing_token":fresh["pairing_token"]})).unwrap();
    let restored = db
        .authenticate("phone", "phone", "android", field(&claim, "device_token"))
        .unwrap();
    assert_eq!(
        restored.scopes,
        vec![
            "status.read",
            "session.read",
            "session.respond",
            "window.show"
        ]
    );
    assert!(restored.file_browser_roots.is_empty());
    assert!(restored.mirror_sources.is_empty());
    assert!(db
        .authenticate("phone", "phone", "android", &format!("{SECRET}-phone"))
        .is_err());
}

#[test]
fn windows_offline_keeps_mac_snapshots_and_no_mac_history() {
    let (dir, mut db, phone) = setup(8 * 1024 * 1024);
    let p = publication("mirror", 1);
    db.publish("mac", p.clone()).unwrap();
    db.publish("win", publication("mcp", 1)).unwrap();
    drop(db);
    let mut db = Store::open(&dir.path().join("hub.sqlite"), 8 * 1024 * 1024).unwrap();
    let snapshots = db.snapshots(&phone, None).unwrap();
    assert!(snapshots
        .iter()
        .any(|v| v["message_type"] == "cross_device_mirror_state"
            && v["payload"]["request"].is_object()));
    assert!(snapshots
        .iter()
        .all(|v| v["message_type"] != "mcp_state" || v["payload"]["request"].is_null()));
    assert!(db
        .rpc(
            &phone,
            "sync",
            json!({"message_type":"request_sync","payload":{}})
        )
        .unwrap_err()
        .to_string()
        .contains("windows_offline"));
    assert!(db
        .rpc(
            &phone,
            "timeline",
            json!({"payload":{"request_id":p.route_id}})
        )
        .is_err());
}

#[test]
fn image_ids_stay_global_durable_and_session_bound() {
    let (dir, mut db, phone) = setup(8 * 1024 * 1024);
    let mut p = publication("mcp", 1);
    let upload = json!({"epoch":1,"original_asset_id":format!("img_{}","a".repeat(32)),"source_request_id":p.source_request_id,"version":1,"mime":"image/png","data_base64":STANDARD.encode(b"\x89PNG\r\n\x1a\n")});
    let asset = db.image("win", upload.clone()).unwrap();
    let id = field(&asset, "id").to_string();
    assert!(image_id(&id));
    p.wire["payload"]["request"]["message"] = json!(format!("image ![proof](/image?id={id})"));
    db.publish("win", p.clone()).unwrap();
    assert!(db.read_image(&phone, &id).is_ok());
    assert!(db.image("mac", upload.clone()).is_err());
    let mut wrong = phone.clone();
    wrong.windows_source = "other-win".into();
    assert!(db.read_image(&wrong, &id).is_err());
    assert!(db.read_image(&phone, "../../private").is_err());
    let mut p2 = publication("mcp", 2);
    p2.wire["payload"]["request"]["message"] = p.wire["payload"]["request"]["message"].clone();
    assert!(db.publish("win", p2).is_err());
    drop(db);
    let mut db = Store::open(&dir.path().join("hub.sqlite"), 8 * 1024 * 1024).unwrap();
    assert_eq!(db.image("win", upload).unwrap()["id"], id);
    db.touch("win").unwrap();
    assert_eq!(db.read_image(&phone, &id).unwrap().1, b"\x89PNG\r\n\x1a\n");
}

#[test]
fn budget_refuses_new_work_but_can_finish_previously_admitted_action() {
    let (_dir, mut db, phone) = setup(4 * 1024 * 1024);
    let p = publication("mirror", 1);
    db.publish("mac", p.clone()).unwrap();
    db.action(&phone, action(&p, "a")).unwrap();
    let j = db.next("mac").unwrap()["jobs"][0].clone();
    let mut rejected = false;
    for i in 0..100 {
        let v = json!({"epoch":1,"original_asset_id":format!("img_{i:032x}"),"source_request_id":"fill","version":1,"mime":"image/png","data_base64":STANDARD.encode(vec![0u8;60000])});
        if db.image("win", v).is_err() {
            rejected = true;
            break;
        }
    }
    assert!(rejected);
    assert_eq!(
        db.confirm("mac", confirmation(&p, &j, "a")).unwrap()["payload"]["status"],
        "accepted"
    );
    assert_eq!(
        db.action(&phone, action(&p, "a")).unwrap()["payload"]["status"],
        "accepted"
    );
}

#[test]
fn freeze_epoch_requires_drain_and_every_source_barrier() {
    let (_dir, mut db, phone) = setup(8 * 1024 * 1024);
    assert_eq!(db.next("mac").unwrap()["drain"], false);
    db.control(json!({"frozen":true,"drain":true})).unwrap();
    assert_eq!(db.next("mac").unwrap()["drain"], true);
    db.control(json!({"frozen":false})).unwrap();
    assert_eq!(db.next("mac").unwrap()["drain"], false);
    let p = publication("mirror", 1);
    db.publish("mac", p.clone()).unwrap();
    db.action(&phone, action(&p, "a")).unwrap();
    db.control(json!({"frozen":true})).unwrap();
    assert!(db.action(&phone, action(&p, "new")).is_err());
    assert!(db.next("mac").unwrap()["jobs"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(db.control(json!({"advance_epoch":true})).is_err());
    db.control(json!({"frozen":false})).unwrap();
    let j = db.next("mac").unwrap()["jobs"][0].clone();
    db.confirm("mac", confirmation(&p, &j, "a")).unwrap();
    db.control(json!({"frozen":true})).unwrap();
    assert!(db.control(json!({"advance_epoch":true})).is_err());
    for s in ["win", "mac"] {
        let generation = db.next(s).unwrap();
        db.barrier(
            s,
            json!({"epoch":1,"freeze_generation":generation["freeze_generation"],"source_generation":generation["source_generation"],"legacy_connections_closed":true,"all_submit_paths_paused":true}),
        )
        .unwrap();
    }
    assert_eq!(
        db.control(json!({"advance_epoch":true})).unwrap()["epoch"],
        2
    );
}

#[test]
fn windows_sync_native_history_and_timeline_route_owner_are_persistent() {
    let (_dir, mut db, phone) = setup(8 * 1024 * 1024);
    db.touch("win").unwrap();
    let id = db
        .rpc(
            &phone,
            "sync",
            json!({"message_type":"request_sync","payload":{"sync_reason":"android_connect:test"}}),
        )
        .unwrap();
    db.next("win").unwrap();
    let response = json!({"events":[{"message_type":"native_history_sessions","payload":{"source":"codex_native","sessions":[{"routeId":"native-cli:old","projectPath":"C:\\Allowed","title":"history"}]}},{"message_type":"mcp_state","payload":{"sync_response":true,"sync_reason":"android_connect:test","customPrompts":{"enabled":true,"prompts":[{"id":"p1","name":"continue","content":"go"}]}}}]});
    db.rpc_response("win", json!({"job_id":id,"epoch":1,"response":response}))
        .unwrap();
    assert!(db
        .snapshots(&phone, Some("android_connect:test"))
        .unwrap()
        .iter()
        .any(|v| v["message_type"] == "native_history_sessions"));
    assert!(db.rpc(&phone,"timeline",json!({"message_type":"request_timeline_sync","payload":{"request_id":"native-cli:old","project_path":"C:\\Allowed"}})).is_ok());
    assert!(db
        .rpc(
            &phone,
            "timeline",
            json!({"payload":{"request_id":"unknown","project_path":"C:\\Allowed"}})
        )
        .is_err());
}

#[test]
fn drain_freezes_new_admission_and_retains_source_final_authorization() {
    let (_dir, mut db, phone) = setup(8 * 1024 * 1024);
    let p = publication("mirror", 1);
    db.publish("mac", p.clone()).unwrap();
    db.action(&phone, action(&p, "a")).unwrap();
    db.control(json!({"frozen":true,"drain":true})).unwrap();
    assert!(db.action(&phone, action(&p, "b")).is_err());
    let j = db.next("mac").unwrap()["jobs"][0].clone();
    let jid = field(&j, "id");
    assert!(db.authorize_job("mac", jid).is_ok());
    assert!(db.authorize_job("win", jid).is_err());
    db.revoke("phone", "phone").unwrap();
    assert!(db.authorize_job("mac", jid).is_err());
}

#[test]
fn main_page_and_open_codex_are_scoped_to_windows_and_source_confirmed() {
    let (_dir, mut db, phone) = setup(8 * 1024 * 1024);
    assert!(db.rpc(&phone, "page", json!({"path":"/"})).is_err());
    db.touch("win").unwrap();
    let id = db
        .rpc(&phone, "page", json!({"method":"GET","path":"/"}))
        .unwrap();
    assert_eq!(db.next("mac").unwrap()["jobs"].as_array().unwrap().len(), 0);
    let j = db.next("win").unwrap()["jobs"][0].clone();
    assert_eq!(j["kind"], "page");
    assert_eq!(
        j["payload"]["principal"]["file_browser_roots"],
        json!(["C:\\Allowed"])
    );
    assert!(db.rpc_result(&phone, &id).unwrap().is_none());
    assert!(db
        .rpc_response(
            "mac",
            json!({"job_id":id,"epoch":1,"response":{"status":200,"body":"wrong source"}})
        )
        .is_err());
    db.rpc_response("win",json!({"job_id":id,"epoch":1,"response":{"status":200,"body":"<html>Windows content</html>"}})).unwrap();
    assert_eq!(db.rpc_result(&phone, &id).unwrap().unwrap()["status"], 200);
    let id = db
        .rpc(&phone, "open_codex", json!({"project_path":"C:\\Allowed"}))
        .unwrap();
    db.next("win").unwrap();
    db.rpc_response("win", json!({"job_id":id,"epoch":1,"response":{"ok":true}}))
        .unwrap();
    assert_eq!(db.rpc_result(&phone, &id).unwrap().unwrap()["ok"], true);
}

#[test]
fn source_receipt_closes_active_snapshot_without_claiming_ai_continuation() {
    let (_dir, mut db, phone) = setup(8 * 1024 * 1024);
    let p = publication("mirror", 1);
    db.publish("mac", p.clone()).unwrap();
    db.action(&phone, action(&p, "a")).unwrap();
    let j = db.next("mac").unwrap()["jobs"][0].clone();
    db.confirm("mac", confirmation(&p, &j, "a")).unwrap();
    let snapshots = db.snapshots(&phone, None).unwrap();
    assert!(snapshots
        .iter()
        .any(|v| v["message_type"] == "cross_device_mirror_state"
            && v["payload"]["closed"] == true
            && v["payload"]["request"].is_null()));
    assert_eq!(
        db.status().unwrap()["actions"][0]["state"],
        "source_accepted"
    );
}
