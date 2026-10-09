use crate::protocol::*;
use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use subtle::ConstantTimeEq;

mod desktop;

pub struct Store {
    conn: Connection,
    budget: u64,
    boot: String,
}

fn key<T: Serialize>(v: T) -> String {
    serde_json::to_string(&v).expect("serializable identity")
}
// Match the source client's bound, including the entire JSON envelope.
const MAX_SOURCE_RESPONSE: usize = 2 * 1024 * 1024;

fn register_timeline(
    tx: &Transaction<'_>,
    budget: u64,
    source: &str,
    p: &Publication,
) -> Result<()> {
    if p.kind == "mirror" {
        return Ok(());
    }
    let explicit = field(&p.wire["payload"], "timeline_route_id");
    let nested = field(&p.wire["payload"]["request"], "timeline_route_id");
    let route = if !explicit.is_empty() {
        explicit
    } else if !nested.is_empty() {
        nested
    } else if p.kind == "mcp" {
        &p.route_id
    } else {
        return Ok(());
    };
    let routekey = key((route, &p.project_path));
    if let Some(old) = get::<String>(tx, "timeline", &routekey)? {
        ensure!(old == source, "timeline_source_conflict");
    }
    put(tx, budget, "timeline", &routekey, &source)
}
fn get<T: DeserializeOwned>(db: &Connection, kind: &str, id: &str) -> Result<Option<T>> {
    let raw: Option<String> = db
        .query_row(
            "SELECT value FROM objects WHERE kind=?1 AND id=?2",
            params![kind, id],
            |r| r.get(0),
        )
        .optional()?;
    raw.map(|s| serde_json::from_str(&s).map_err(Into::into))
        .transpose()
}
fn all<T: DeserializeOwned>(db: &Connection, kind: &str) -> Result<Vec<T>> {
    let mut stmt = db.prepare("SELECT value FROM objects WHERE kind=?1 ORDER BY id")?;
    let rows = stmt.query_map([kind], |r| r.get::<_, String>(0))?;
    rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
}
// Every persisted byte is admitted transactionally. Existing reservations allow
// acknowledgements to finish when new work is refused. No automatic deletion.
fn put<T: Serialize>(
    tx: &Transaction<'_>,
    budget: u64,
    kind: &str,
    id: &str,
    value: &T,
) -> Result<()> {
    let raw = serde_json::to_string(value)?;
    let old: u64 = tx
        .query_row(
            "SELECT charge FROM objects WHERE kind=?1 AND id=?2",
            params![kind, id],
            |r| r.get::<_, i64>(0).map(|n| n as u64),
        )
        .optional()?
        .unwrap_or(0);
    let reserve = match kind {
        "action" | "job" => MAX_MESSAGE as u64 + 4096,
        "desktop_relay" => MAX_SOURCE_RESPONSE as u64 + 4096,
        _ => 4096,
    };
    let needed = raw.len() as u64 + kind.len() as u64 + id.len() as u64;
    let charge = if old > 0 {
        old.max(needed)
    } else {
        needed + reserve
    };
    let used: u64 = tx.query_row("SELECT COALESCE(SUM(charge),0) FROM objects", [], |r| {
        r.get::<_, i64>(0).map(|n| n as u64)
    })?;
    ensure!(used - old + charge <= budget * 4 / 5, "capacity_exceeded");
    tx.execute("INSERT INTO objects(kind,id,value,charge) VALUES(?1,?2,?3,?4) ON CONFLICT(kind,id) DO UPDATE SET value=excluded.value,charge=excluded.charge",params![kind,id,raw,charge as i64])?;
    Ok(())
}
fn meta(db: &Connection) -> Result<Value> {
    Ok(get(db, "meta", "control")?.context("missing_control")?)
}
fn admit(db: &Connection) -> Result<u64> {
    let m = meta(db)?;
    ensure!(m["frozen"] == false, "route_frozen");
    Ok(m["epoch"].as_u64().context("bad_epoch")?)
}
fn scope(phone: &Credential, required: &str) -> Result<()> {
    ensure!(phone.scope(required), "scope_denied");
    Ok(())
}
fn can_read(phone: &Credential, req: &RequestRecord) -> bool {
    phone.scope("session.read")
        && if req.publication.kind == "mirror" {
            phone.mirror_sources.contains(&req.source)
        } else {
            phone.windows_source == req.source
        }
}
fn source_credential(db: &Connection, id: &str) -> Result<Credential> {
    let c: Credential = get(db, "source", id)?.context("unknown_source")?;
    ensure!(c.revoked_at.is_none(), "source_revoked");
    Ok(c)
}
fn job(
    tx: &Transaction<'_>,
    budget: u64,
    source: &str,
    kind: &str,
    payload: Value,
    epoch: u64,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let seq: i64 = tx.query_row("SELECT next_seq FROM sequence WHERE id=1", [], |r| r.get(0))?;
    tx.execute("UPDATE sequence SET next_seq=next_seq+1 WHERE id=1", [])?;
    put(
        tx,
        budget,
        "job",
        &id,
        &json!({"id":id,"seq":seq,"source":source,"kind":kind,"payload":payload,"epoch":epoch,"state":"queued"}),
    )?;
    Ok(id)
}

impl Store {
    pub fn open(path: &Path, budget: u64) -> Result<Self> {
        ensure!(
            (128 * 1024..=MAX_BUDGET).contains(&budget),
            "invalid_budget"
        );
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let schema_version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(schema_version <= 1, "unsupported_database_version");
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
          CREATE TABLE IF NOT EXISTS objects(kind TEXT NOT NULL,id TEXT NOT NULL,value TEXT NOT NULL CHECK(json_valid(value)),charge INTEGER NOT NULL CHECK(charge>=0),PRIMARY KEY(kind,id));
          CREATE TABLE IF NOT EXISTS sequence(id INTEGER PRIMARY KEY CHECK(id=1),next_seq INTEGER NOT NULL);
          INSERT OR IGNORE INTO sequence VALUES(1,1);
          PRAGMA user_version=1;")?;
        let pages: u64 = conn.query_row("PRAGMA page_count", [], |r| {
            r.get::<_, i64>(0).map(|n| n as u64)
        })?;
        let page_size: u64 = conn.query_row("PRAGMA page_size", [], |r| {
            r.get::<_, i64>(0).map(|n| n as u64)
        })?;
        ensure!(pages * page_size <= budget, "database_over_budget");
        conn.pragma_update(None, "max_page_count", (budget / page_size) as i64)?;
        if get::<Value>(&conn, "meta", "control")?.is_none() {
            let tx = conn.transaction()?;
            put(
                &tx,
                budget,
                "meta",
                "control",
                &json!({"epoch":1,"frozen":false,"freeze_generation":0}),
            )?;
            tx.commit()?;
        }
        Ok(Self {
            conn,
            budget,
            boot: uuid::Uuid::new_v4().to_string(),
        })
    }

    pub fn authenticate(
        &self,
        kind: &str,
        id: &str,
        client_kind: &str,
        token: &str,
    ) -> Result<Credential> {
        ensure!(
            identifier(id) && token.len() >= 20 && token.len() <= 512,
            "invalid_credential"
        );
        let c: Credential = get(&self.conn, kind, id)?.context("invalid_credential")?;
        ensure!(
            c.revoked_at.is_none() && c.client_kind != "ios" && c.client_kind == client_kind,
            "invalid_credential"
        );
        ensure!(
            bool::from(
                hash(token.as_bytes())
                    .as_bytes()
                    .ct_eq(c.token_hash.as_bytes())
            ),
            "invalid_credential"
        );
        Ok(c)
    }

    pub fn provision(&mut self, kind: &str, c: Credential) -> Result<()> {
        ensure!(
            matches!(kind, "source" | "phone")
                && identifier(&c.device_id)
                && hash_valid(&c.token_hash),
            "invalid_provision"
        );
        ensure!(
            c.scopes.len() <= 32
                && c.file_browser_roots.len() <= 128
                && c.mirror_sources.len() <= 32,
            "invalid_provision"
        );
        if kind == "source" {
            ensure!(
                matches!(c.client_kind.as_str(), "windows" | "macos"),
                "invalid_source_platform"
            );
        } else {
            ensure!(
                matches!(c.client_kind.as_str(), "android" | "ios"),
                "invalid_phone_kind"
            );
            ensure!(
                source_credential(&self.conn, &c.windows_source)?.client_kind == "windows",
                "invalid_windows_source"
            );
            for source in &c.mirror_sources {
                ensure!(
                    source_credential(&self.conn, source)?.client_kind == "macos",
                    "invalid_mirror_source"
                );
            }
        }
        // Migration is immutable: retries must be identical. Revoke is separate;
        // widening grants requires an explicitly new provisioned identity.
        if let Some(old) = get::<Credential>(&self.conn, kind, &c.device_id)? {
            ensure!(
                serde_json::to_value(old)? == serde_json::to_value(&c)?,
                "credential_import_conflict"
            );
            return Ok(());
        }
        let tx = self.conn.transaction()?;
        admit(&tx)?;
        put(&tx, self.budget, kind, &c.device_id, &c)?;
        if kind == "source" {
            put(
                &tx,
                self.budget,
                "cursor",
                &c.device_id,
                &json!({"acknowledged":0}),
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn revoke(&mut self, kind: &str, id: &str) -> Result<()> {
        ensure!(matches!(kind, "source" | "phone"), "invalid_kind");
        let tx = self.conn.transaction()?;
        let mut c: Credential = get(&tx, kind, id)?.context("unknown_device")?;
        c.revoked_at = Some(chrono::Utc::now().to_rfc3339());
        put(&tx, self.budget, kind, id, &c)?;
        tx.commit()?;
        Ok(())
    }

    pub fn pairing(&mut self, mut grant: Credential) -> Result<Value> {
        // Only fresh short-lived QR capabilities are issued; no old QR import.
        grant.device_id = String::new();
        grant.token_hash = String::new();
        grant.revoked_at = None;
        ensure!(grant.client_kind == "android", "invalid_phone_kind");
        ensure!(
            source_credential(&self.conn, &grant.windows_source)?.client_kind == "windows",
            "invalid_windows_source"
        );
        for s in &grant.mirror_sources {
            ensure!(
                source_credential(&self.conn, s)?.client_kind == "macos",
                "invalid_mirror_source"
            );
        }
        let token = token("pt");
        let id = uuid::Uuid::new_v4().to_string();
        let tx = self.conn.transaction()?;
        admit(&tx)?;
        put(
            &tx,
            self.budget,
            "pairing",
            &hash(token.as_bytes()),
            &json!({"session_id":id,"expires":now()+120,"used":false,"grant":grant,"source_applied":false}),
        )?;
        put(
            &tx,
            self.budget,
            "pairing_session",
            &id,
            &hash(token.as_bytes()),
        )?;
        tx.commit()?;
        Ok(
            json!({"version":2,"pairing_session_id":id,"device_id":grant.windows_source,"device_name":"iterate hub","transport_mode":"public_tunnel","base_url":PUBLIC_URL,"ws_url":"wss://8.129.82.226:8443/ws","candidates":[{"transport_mode":"public_tunnel","base_url":PUBLIC_URL,"ws_url":"wss://8.129.82.226:8443/ws","health":"healthy","disabled":false,"warning":null}],"pairing_token":token,"issued_at":chrono::Utc::now().to_rfc3339(),"expires_at":chrono::DateTime::from_timestamp(now()+120,0).unwrap().to_rfc3339(),"warning":null}),
        )
    }

    pub fn source_pairing(&mut self, source: &Credential) -> Result<Value> {
        ensure!(
            source.client_kind == "windows" && source.scope("pairing.issue"),
            "pairing_scope_denied"
        );
        self.pairing(Credential {
            device_id: String::new(),
            client_kind: "android".into(),
            token_hash: String::new(),
            scopes: [
                "status.read",
                "session.read",
                "session.respond",
                "window.show",
            ]
            .map(str::to_owned)
            .to_vec(),
            revoked_at: None,
            file_browser_roots: vec![],
            windows_source: source.device_id.clone(),
            mirror_sources: vec![],
        })
    }

    pub fn source_pairing_session(&self, source: &Credential, id: &str) -> Result<Value> {
        ensure!(
            source.client_kind == "windows" && source.scope("pairing.issue"),
            "pairing_scope_denied"
        );
        let key: String =
            get(&self.conn, "pairing_session", id)?.context("unknown_pairing_session")?;
        let p: Value = get(&self.conn, "pairing", &key)?.context("unknown_pairing_session")?;
        ensure!(
            p["grant"]["windows_source"] == source.device_id,
            "source_permission_denied"
        );
        let mut credential = p["credential"].clone();
        let mut superseded = false;
        if !credential.is_null() {
            let current: Credential = get(&self.conn, "phone", field(&credential, "device_id"))?
                .context("unknown_phone")?;
            // Never let an older local intent replace a later re-pair/revoke.
            superseded =
                current.revoked_at.is_some() || current.token_hash != credential["token_hash"];
            credential = if superseded {
                Value::Null
            } else {
                serde_json::to_value(current)?
            };
        }
        let state = if superseded {
            "failed"
        } else if p["used"] == true {
            if p["source_applied"] == true && p["connected_at"].is_string() {
                "connected"
            } else {
                "claimed"
            }
        } else if p["expires"].as_i64().unwrap_or(0) <= now() {
            "expired"
        } else {
            "pending"
        };
        Ok(
            json!({"session":{"session_id":id,"state":state,"expires_at":chrono::DateTime::from_timestamp(p["expires"].as_i64().unwrap_or(0),0).context("invalid_pairing_expiry")?.to_rfc3339(),
            "device_id":credential["device_id"],"device_name":p["device_name"],"client_kind":credential["client_kind"],"claimed_at":p["claimed_at"],"connected_at":p["connected_at"],
            "selected_transport_mode":if state=="connected"{json!("public_tunnel")}else{Value::Null}},"credential":credential}),
        )
    }

    pub fn source_pairing_applied(&mut self, source: &Credential, v: &Value) -> Result<()> {
        let id = field(v, "session_id");
        let snapshot = self.source_pairing_session(source, id)?;
        ensure!(
            snapshot["credential"].is_object()
                && snapshot["credential"]["token_hash"] == v["token_hash"],
            "pairing_credential_mismatch"
        );
        let tx = self.conn.transaction()?;
        let key: String = get(&tx, "pairing_session", id)?.context("unknown_pairing_session")?;
        let mut p: Value = get(&tx, "pairing", &key)?.context("unknown_pairing_session")?;
        p["source_applied"] = json!(true);
        put(&tx, self.budget, "pairing", &key, &p)?;
        tx.commit()?;
        Ok(())
    }

    pub fn pairing_connected(&mut self, phone: &Credential, transport: &str) -> Result<()> {
        if transport != "public_tunnel" {
            return Ok(());
        }
        let Some(key) = get::<String>(&self.conn, "phone_pairing", &phone.device_id)? else {
            return Ok(());
        };
        let tx = self.conn.transaction()?;
        let mut p: Value = get(&tx, "pairing", &key)?.context("unknown_pairing_session")?;
        if p["credential"]["token_hash"] == phone.token_hash && p["connected_at"].is_null() {
            p["connected_at"] = json!(chrono::Utc::now().to_rfc3339());
            put(&tx, self.budget, "pairing", &key, &p)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn claim(&mut self, v: Value) -> Result<Value> {
        ensure!(
            field(&v, "client_kind") == "android" && identifier(field(&v, "device_id")),
            "invalid_claim"
        );
        let token_raw = field(&v, "pairing_token");
        ensure!(
            token_raw.starts_with("pt_") && token_raw.len() < 512,
            "invalid_pairing"
        );
        let id = hash(token_raw.as_bytes());
        let tx = self.conn.transaction()?;
        admit(&tx)?;
        let mut pair: Value = get(&tx, "pairing", &id)?.context("invalid_pairing")?;
        ensure!(
            pair["used"] == false && pair["expires"].as_i64().unwrap_or(0) > now(),
            "expired_or_used_pairing"
        );
        let mut c: Credential = serde_json::from_value(pair["grant"].clone())?;
        c.device_id = field(&v, "device_id").into();
        if let Some(old) = get::<Credential>(&tx, "phone", &c.device_id)? {
            ensure!(old.client_kind == "android", "device_id_retired_ios");
            ensure!(
                old.windows_source == c.windows_source,
                "source_permission_denied"
            );
            if old.revoked_at.is_none() {
                c = old;
            } else {
                // Explicitly scanning a fresh desktop QR reauthorizes Android
                // only. Migration never clears revocation; old roots and Mac
                // permissions do not revive through this Windows capability.
                c.file_browser_roots.clear();
                c.mirror_sources.clear();
            }
        }
        let raw = token("dt");
        c.token_hash = hash(raw.as_bytes());
        put(&tx, self.budget, "phone", &c.device_id, &c)?;
        pair["used"] = json!(true);
        pair["credential"] = serde_json::to_value(&c)?;
        pair["device_name"] = json!(field(&v, "device_name")
            .chars()
            .take(128)
            .collect::<String>());
        pair["claimed_at"] = json!(chrono::Utc::now().to_rfc3339());
        put(&tx, self.budget, "phone_pairing", &c.device_id, &id)?;
        put(&tx, self.budget, "pairing", &id, &pair)?;
        tx.commit()?;
        Ok(
            json!({"ok":true,"device_id":c.device_id,"device_token":raw,"scopes":c.scopes,"pairing_session_id":pair["session_id"]}),
        )
    }

    pub fn touch(&mut self, source: &str) -> Result<()> {
        let live: Value = get(&self.conn, "live", source)?.unwrap_or(json!({}));
        let session = if field(&live, "session").is_empty() {
            "internal"
        } else {
            field(&live, "session")
        };
        self.touch_session(source, session)
    }

    pub fn touch_session(&mut self, source: &str, session: &str) -> Result<()> {
        source_credential(&self.conn, source)?;
        ensure!(identifier(session), "source_session_required");
        let tx = self.conn.transaction()?;
        let live: Value = get(&tx, "live", source)?.unwrap_or(json!({}));
        let previous = live["generation"].as_u64().unwrap_or(0);
        let same = live["boot"] == self.boot
            && live["session"] == session
            && live["at"].as_i64().unwrap_or(0) + 45 >= now();
        let generation = if same {
            previous
        } else {
            previous
                .checked_add(1)
                .context("source_generation_exhausted")?
        };
        put(
            &tx,
            self.budget,
            "live",
            source,
            &json!({"boot":self.boot,"at":now(),"session":session,"generation":generation}),
        )?;
        tx.commit()?;
        Ok(())
    }
    fn online(&self, source: &str) -> Result<bool> {
        Ok(source_credential(&self.conn, source).is_ok()
            && get::<Value>(&self.conn, "live", source)?.is_some_and(|v| {
                v["boot"] == self.boot && v["at"].as_i64().unwrap_or(0) + 45 >= now()
            }))
    }

    pub fn publish(&mut self, source: &str, mut p: Publication) -> Result<()> {
        let c = source_credential(&self.conn, source)?;
        ensure!(
            p.published
                && p.version > 0
                && identifier(&p.source_request_id)
                && identifier(&p.registration_key)
                && identifier(&p.route_id)
                && p.wire.to_string().len() <= MAX_MESSAGE,
            "invalid_publication"
        );
        let payload = &p.wire["payload"];
        ensure!(
            payload.is_object() && payload["request"].is_object(),
            "only_published_requests"
        );
        let request = &payload["request"];
        if p.kind == "mirror" {
            ensure!(
                c.client_kind == "macos" && p.project_path.is_empty() && mirror_id(&p.route_id),
                "invalid_mirror_identity"
            );
            ensure!(
                field(&p.wire, "message_type") == "cross_device_mirror_state"
                    && field(payload, "source") == "cross_device_mirror"
                    && field(payload, "mirror_id") == p.route_id
                    && field(payload, "request_id") == p.route_id
                    && field(request, "id") == p.route_id,
                "invalid_mirror_wire"
            );
            ensure!(
                [payload, request]
                    .iter()
                    .all(|v| field(v, "project_path").is_empty()
                        && field(v, "projectPath").is_empty()),
                "mirror_project_forbidden"
            );
            ensure!(
                !field(request, "message").is_empty()
                    && !request
                        .get("images")
                        .is_some_and(|v| v.as_array().is_some_and(|a| !a.is_empty()))
                    && !field(request, "message").contains("/image?id="),
                "mirror_text_only"
            );
            ensure!(
                request.as_object().unwrap().keys().all(|k| matches!(
                    k.as_str(),
                    "id" | "message"
                        | "predefined_options"
                        | "conversation_title"
                        | "is_markdown"
                        | "project_path"
                        | "projectPath"
                )),
                "mirror_field_forbidden"
            );
        } else {
            ensure!(
                c.client_kind == "windows"
                    && matches!(p.kind.as_str(), "mcp" | "native")
                    && !p.project_path.is_empty(),
                "invalid_windows_request"
            );
            ensure!(
                field(&p.wire, "message_type") == "mcp_state",
                "invalid_request_wire"
            );
            let rid = ["request_id", "requestId", "id"]
                .iter()
                .map(|k| field(request, k))
                .find(|v| !v.is_empty())
                .unwrap_or("");
            let project = ["project_path", "projectPath"]
                .iter()
                .map(|k| field(request, k))
                .find(|v| !v.is_empty())
                .unwrap_or("");
            ensure!(
                rid == p.route_id && project == p.project_path,
                "wire_route_mismatch"
            );
            ensure!(
                (p.kind == "native") == p.route_id.starts_with("native-codex:"),
                "native_kind_mismatch"
            );
            let source_kind = if field(payload, "source").is_empty() {
                field(request, "source")
            } else {
                field(payload, "source")
            };
            ensure!(
                (p.kind == "native") == (source_kind == "codex_native")
                    && !p.route_id.starts_with("cross-mirror:")
                    && source_kind != "cross_device_mirror",
                "reserved_source"
            );
        }
        // Presentation identity comes from the authenticated source, never the wire.
        ensure!(
            field(payload, "origin_device_id").is_empty()
                || field(payload, "origin_device_id") == source,
            "wrong_origin"
        );
        ensure!(
            field(payload, "origin_platform").is_empty()
                || field(payload, "origin_platform") == c.client_kind,
            "wrong_platform"
        );
        self.validate_images(source, &p)?;
        p.wire["payload"]["origin_device_id"] = json!(source);
        p.wire["payload"]["origin_platform"] = json!(c.client_kind);
        let identity = key((source, &p.source_request_id, p.version));
        let tx = self.conn.transaction()?;
        if let Some(old) = get::<RequestRecord>(&tx, "request", &identity)? {
            ensure!(
                serde_json::to_value(&old.publication)? == serde_json::to_value(&p)?,
                "publication_conflict"
            );
            register_timeline(&tx, self.budget, source, &old.publication)?;
            tx.commit()?;
            return Ok(());
        }
        ensure!(p.epoch == admit(&tx)?, "stale_epoch");
        // Android carries no version field: a route can NEVER be rebound to a
        // different request/version. Adapters must allocate a fresh wire route.
        ensure!(
            get::<String>(&tx, "route", &p.route_id)?.is_none(),
            "route_reuse_forbidden"
        );
        let latest_key = key((source, &p.source_request_id));
        if let Some(latest) = get::<u64>(&tx, "latest", &latest_key)? {
            ensure!(p.version > latest, "stale_version");
            let previous = key((source, &p.source_request_id, latest));
            let mut old: RequestRecord =
                get(&tx, "request", &previous)?.context("missing_request")?;
            old.active = false;
            put(&tx, self.budget, "request", &previous, &old)?;
        }
        let mut record = RequestRecord {
            source: source.into(),
            publication: p,
            active: true,
            winner_action: None,
        };
        record.publication.wire["payload"]["origin_device_id"] = json!(source);
        record.publication.wire["payload"]["origin_platform"] = json!(c.client_kind);
        put(
            &tx,
            self.budget,
            "latest",
            &latest_key,
            &record.publication.version,
        )?;
        put(
            &tx,
            self.budget,
            "route",
            &record.publication.route_id,
            &identity,
        )?;
        // Timeline routes are also owned by one source, never broadcast fallback.
        register_timeline(&tx, self.budget, source, &record.publication)?;
        put(&tx, self.budget, "request", &identity, &record)?;
        tx.commit()?;
        self.touch(source)?;
        Ok(())
    }

    pub fn close_request(&mut self, source: &str, v: Value) -> Result<()> {
        source_credential(&self.conn, source)?;
        let id = key((
            source,
            field(&v, "source_request_id"),
            v["version"].as_u64().context("invalid_version")?,
        ));
        let tx = self.conn.transaction()?;
        let mut r: RequestRecord = get(&tx, "request", &id)?.context("unknown_request")?;
        ensure!(
            r.publication.registration_key == field(&v, "registration_key"),
            "registration_mismatch"
        );
        r.active = false;
        put(&tx, self.budget, "request", &id, &r)?;
        tx.commit()?;
        Ok(())
    }

    pub fn action(&mut self, phone: &Credential, wire: Value) -> Result<Value> {
        scope(phone, "session.read")?;
        scope(phone, "session.respond")?;
        ensure!(wire.to_string().len() <= MAX_MESSAGE, "message_too_large");
        let p = &wire["payload"];
        let client = field(p, "client_action_id");
        ensure!(
            identifier(client)
                && identifier(field(p, "request_id"))
                && identifier(field(p, "action")),
            "invalid_action"
        );
        let lookup = key((&phone.device_id, client));
        let tx = self.conn.transaction()?;
        // native_query is an old Android query-only flag, never a fresh action.
        let query = p["native_query"] == true;
        let mut comparison = wire.clone();
        if query {
            comparison["payload"]
                .as_object_mut()
                .context("invalid_action")?
                .remove("native_query");
        }
        if let Some(id) = get::<String>(&tx, "client_action", &lookup)? {
            let old: ActionRecord = get(&tx, "action", &id)?.context("missing_action")?;
            ensure!(old.wire == comparison, "action_content_conflict");
            ensure!(can_read(phone, &old.request), "source_permission_denied");
            return Ok(receipt(&old));
        }
        ensure!(!query, "unknown_action_query");
        let epoch = admit(&tx)?;
        let reqkey: String =
            get(&tx, "route", field(p, "request_id"))?.context("unknown_request")?;
        let r: RequestRecord = get(&tx, "request", &reqkey)?.context("unknown_request")?;
        ensure!(can_read(phone, &r), "source_permission_denied");
        source_credential(&tx, &r.source)?;
        ensure!(r.active, "stale_version_or_closed");
        ensure!(
            field(p, "project_path") == r.publication.project_path,
            "project_mismatch"
        );
        if r.publication.kind == "mirror" {
            ensure!(
                field(&wire, "message_type") == "cross_device_mirror_action"
                    && field(p, "mirror_id") == r.publication.route_id,
                "mirror_identity_mismatch"
            );
            ensure!(
                matches!(field(p, "action"), "submit" | "close_mirror")
                    && p.get("images").is_none(),
                "mirror_action_forbidden"
            );
        } else {
            ensure!(
                field(&wire, "message_type") == "mcp_action",
                "action_kind_mismatch"
            );
        }
        let id = uuid::Uuid::new_v4().to_string();
        let a = ActionRecord {
            id: id.clone(),
            phone: phone.device_id.clone(),
            source: r.source.clone(),
            request: r,
            wire: comparison,
            state: "queued".into(),
            winner_record: None,
            source_receipt: None,
            original_response: None,
            reason: String::new(),
            epoch,
        };
        let j = job(
            &tx,
            self.budget,
            &a.source,
            "action",
            json!({"action_id":id,"source_request_id":a.request.publication.source_request_id,"version":a.request.publication.version,"registration_key":a.request.publication.registration_key,"wire":a.wire,"principal":phone_context(phone)}),
            epoch,
        )?;
        put(&tx, self.budget, "action_job", &id, &j)?;
        put(&tx, self.budget, "action", &id, &a)?;
        put(&tx, self.budget, "client_action", &lookup, &id)?;
        tx.commit()?;
        Ok(receipt(&a))
    }

    pub fn next(&mut self, source: &str) -> Result<Value> {
        self.touch(source)?;
        let tx = self.conn.transaction()?;
        let m = meta(&tx)?;
        let mut jobs: Vec<Value> = all(&tx, "job")?;
        jobs.sort_by_key(|v| v["seq"].as_i64().unwrap_or(0));
        // Delivered-but-unconfirmed commands are reconciliation ONLY. They are
        // never blindly replayed after an HTTP response loss or hub restart.
        let cursor: Value = get(&tx, "cursor", source)?.unwrap_or(json!({"acknowledged":0}));
        let live: Value = get(&tx, "live", source)?.context("source_offline")?;
        let envelope = json!({"epoch":m["epoch"],"frozen":m["frozen"],"drain":m["drain"] == true,"freeze_generation":m["freeze_generation"].as_u64().unwrap_or(0),"source_generation":live["generation"],"jobs":[],"reconcile":[],"cursor":cursor});
        let mut remaining = MAX_SOURCE_RESPONSE - serde_json::to_vec(&envelope)?.len();
        let after: i64 = get(&tx, "poll_cursor", source)?.unwrap_or(0);
        let pending: Vec<_> = jobs
            .iter()
            .filter(|v| v["source"] == source && v["state"] == "dispatched")
            .collect();
        let mut unknown = Vec::new();
        let mut last = after;
        // Rotate query-only pages: an unresolved first page must not starve
        // later durable receipts, including batches dispatched by older hubs.
        for j in pending
            .iter()
            .filter(|j| j["seq"].as_i64().unwrap_or(0) > after)
            .chain(
                pending
                    .iter()
                    .filter(|j| j["seq"].as_i64().unwrap_or(0) <= after),
            )
        {
            let query = json!({"id":j["id"],"seq":j["seq"],"kind":j["kind"],"payload":j["payload"],"epoch":j["epoch"],"query_only":true});
            let bytes = serde_json::to_vec(&query)?.len() + usize::from(!unknown.is_empty());
            if bytes > remaining || (!unknown.is_empty() && remaining < MAX_SOURCE_RESPONSE / 2) {
                break;
            }
            remaining -= bytes;
            last = j["seq"].as_i64().unwrap_or(0);
            unknown.push(query);
            if unknown.len() == 32 {
                break;
            }
        }
        if !unknown.is_empty() {
            put(&tx, self.budget, "poll_cursor", source, &last)?;
        }
        let mut batch = Vec::new();
        if m["frozen"] == false || m["drain"] == true {
            for mut j in jobs
                .into_iter()
                .filter(|v| v["source"] == source && v["state"] == "queued")
                .take(16)
            {
                ensure!(j["epoch"] == m["epoch"], "job_epoch_mismatch");
                // Revocation invalidates queued work too; source must also
                // authorize at its final ledger boundary after delivery.
                let phoneid = field(&j["payload"]["principal"], "device_id");
                let c: Option<Credential> = get(&tx, "phone", phoneid)?;
                if !c.is_some_and(|c| c.revoked_at.is_none() && c.client_kind == "android") {
                    j["state"] = json!("completed");
                    j["response"] =
                        json!({"ok":false,"error":"authorization_revoked_before_dispatch"});
                    if j["kind"] == "action" {
                        let aid = field(&j["payload"], "action_id");
                        let mut a: ActionRecord =
                            get(&tx, "action", aid)?.context("missing_action")?;
                        a.state = "rejected".into();
                        a.reason = "authorization_revoked_before_dispatch".into();
                        put(&tx, self.budget, "action", aid, &a)?;
                    }
                    put(&tx, self.budget, "job", field(&j, "id"), &j)?;
                    continue;
                }
                j["state"] = json!("dispatched");
                let bytes = serde_json::to_vec(&j)?.len() + usize::from(!batch.is_empty());
                if bytes > remaining {
                    break;
                }
                remaining -= bytes;
                let id = field(&j, "id").to_string();
                put(&tx, self.budget, "job", &id, &j)?;
                if j["kind"] == "action" {
                    let aid = field(&j["payload"], "action_id");
                    let mut a: ActionRecord = get(&tx, "action", aid)?.context("missing_action")?;
                    a.state = "confirmation_unknown".into();
                    put(&tx, self.budget, "action", aid, &a)?;
                }
                batch.push(j);
            }
        }
        let mut response = envelope;
        response["jobs"] = json!(batch);
        response["reconcile"] = json!(unknown);
        ensure!(
            serde_json::to_vec(&response)?.len() <= MAX_SOURCE_RESPONSE,
            "source_response_too_large"
        );
        tx.commit()?;
        Ok(response)
    }

    pub fn confirm(&mut self, source: &str, c: Confirmation) -> Result<Value> {
        ensure!(
            serde_json::to_vec(&c)?.len() <= MAX_MESSAGE,
            "confirmation_too_large"
        );
        source_credential(&self.conn, source)?;
        let tx = self.conn.transaction()?;
        let mut a: ActionRecord = get(&tx, "action", &c.action_id)?.context("unknown_action")?;
        ensure!(
            a.source == source
                && a.request.publication.source_request_id == c.source_request_id
                && a.request.publication.version == c.version
                && a.request.publication.registration_key == c.registration_key,
            "source_ledger_mismatch"
        );
        ensure!(c.epoch == a.epoch, "stale_epoch");
        let jid: String = get(&tx, "action_job", &a.id)?.context("missing_job")?;
        let mut j: Value = get(&tx, "job", &jid)?.context("missing_job")?;
        ensure!(
            j["state"] == "dispatched" || j["state"] == "completed",
            "action_not_dispatched"
        );
        if matches!(
            a.state.as_str(),
            "source_accepted" | "response_handoff" | "rejected"
        ) {
            if c.state == "response_handoff" && a.state == "source_accepted" {
                ensure!(
                    c.durable && c.winner_record == a.winner_record,
                    "winner_mismatch"
                );
                a.state = c.state;
                put(&tx, self.budget, "action", &a.id, &a)?;
                tx.commit()?;
                return Ok(receipt(&a));
            }
            ensure!(
                c.receipt == a.source_receipt
                    && c.winner_record == a.winner_record
                    && c.original_response == a.original_response,
                "confirmation_conflict"
            );
            return Ok(receipt(&a));
        }
        match c.state.as_str() {
            "unknown" => {}
            "not_executed" => {
                ensure!(
                    c.durable && c.definitely_not_executed,
                    "reconciliation_proof_required"
                );
                a.state = "queued".into();
                j["state"] = json!("queued");
            }
            "source_accepted" | "rejected" => {
                ensure!(c.durable, "source_durable_confirmation_required");
                if c.state == "source_accepted" {
                    ensure!(
                        c.winner_record.as_deref().is_some_and(identifier)
                            && c.original_response.is_some(),
                        "winner_record_required"
                    );
                    let reqkey = key((source, &c.source_request_id, c.version));
                    let mut request: RequestRecord =
                        get(&tx, "request", &reqkey)?.context("unknown_request")?;
                    if let Some(previous) = &request.winner_action {
                        let winner: ActionRecord =
                            get(&tx, "action", previous)?.context("unknown_winner")?;
                        ensure!(
                            winner.winner_record == c.winner_record
                                && winner.original_response == c.original_response,
                            "source_winner_conflict"
                        );
                    } else {
                        request.winner_action = Some(a.id.clone());
                    }
                    request.active = false;
                    put(&tx, self.budget, "request", &reqkey, &request)?;
                }
                let ack = c.receipt.as_ref().context("source_receipt_required")?;
                validate_receipt(&a, ack, c.state == "source_accepted")?;
                ensure!(ack.to_string().len() <= MAX_MESSAGE, "receipt_too_large");
                a.state = c.state;
                a.winner_record = c.winner_record;
                a.source_receipt = c.receipt;
                a.original_response = c.original_response;
                j["state"] = json!("completed");
                let cursor: Value =
                    get(&tx, "cursor", source)?.unwrap_or(json!({"acknowledged":0}));
                put(
                    &tx,
                    self.budget,
                    "cursor",
                    source,
                    &json!({"acknowledged":cursor["acknowledged"].as_i64().unwrap_or(0).max(j["seq"].as_i64().unwrap_or(0))}),
                )?;
            }
            _ => bail!("invalid_confirmation_state"),
        }
        put(&tx, self.budget, "job", &jid, &j)?;
        put(&tx, self.budget, "action", &a.id, &a)?;
        tx.commit()?;
        Ok(receipt(&a))
    }

    pub fn snapshots(&self, phone: &Credential, sync_reason: Option<&str>) -> Result<Vec<Value>> {
        self.snapshots_scoped(phone, sync_reason, None)
    }

    pub fn snapshots_for_session(
        &self,
        phone: &Credential,
        sync_reason: Option<&str>,
        session: &str,
    ) -> Result<Vec<Value>> {
        self.snapshots_scoped(phone, sync_reason, Some(session))
    }

    fn snapshots_scoped(
        &self,
        phone: &Credential,
        sync_reason: Option<&str>,
        session: Option<&str>,
    ) -> Result<Vec<Value>> {
        scope(phone, "session.read")?;
        let mut output = Vec::new();
        for r in all::<RequestRecord>(&self.conn, "request")? {
            if !can_read(phone, &r) || source_credential(&self.conn, &r.source).is_err() {
                continue;
            }
            if r.active && (r.publication.kind == "mirror" || self.online(&r.source)?) {
                output.push(r.publication.wire.clone());
            } else {
                output.push(closed(&r));
            }
        }
        for a in all::<ActionRecord>(&self.conn, "action")? {
            if a.phone == phone.device_id && can_read(phone, &a.request) {
                output.push(receipt(&a));
            }
        }
        for j in all::<Value>(&self.conn, "job")? {
            if j["kind"] == "sync"
                && j["state"] == "completed"
                && j["payload"]["principal"]["device_id"] == phone.device_id
                && self.online(field(&j, "source"))?
            {
                if let Some(session) = session {
                    if field(&j["payload"], "phone_session") != session {
                        continue;
                    }
                }
                // Only request_sync templates/native-state use sync_reason.
                // Timeline replies are scoped to their requesting WS session,
                // phone, route and project by the persisted original job.
                if j["payload"]["operation"] == "sync" {
                    if let Some(reason) = sync_reason {
                        if field(&j["payload"]["wire"]["payload"], "sync_reason") != reason {
                            continue;
                        }
                    }
                }
                if let Some(events) = j["response"]["events"].as_array() {
                    output.extend(events.iter().cloned());
                }
            }
        }
        Ok(output)
    }

    pub fn rpc(&mut self, phone: &Credential, operation: &str, wire: Value) -> Result<String> {
        self.rpc_scoped(phone, operation, wire, None)
    }

    pub fn rpc_for_session(
        &mut self,
        phone: &Credential,
        operation: &str,
        wire: Value,
        session: &str,
    ) -> Result<String> {
        self.rpc_scoped(phone, operation, wire, Some(session))
    }

    fn rpc_scoped(
        &mut self,
        phone: &Credential,
        operation: &str,
        wire: Value,
        session: Option<&str>,
    ) -> Result<String> {
        ensure!(wire.to_string().len() <= MAX_MESSAGE, "message_too_large");
        ensure!(
            matches!(operation, "open_codex" | "sync" | "timeline" | "page"),
            "unsupported_operation"
        );
        scope(
            phone,
            if operation == "open_codex" {
                "window.show"
            } else {
                "session.read"
            },
        )?;
        let source = if operation == "timeline" {
            let p = &wire["payload"];
            let route = field(p, "request_id");
            ensure!(!route.starts_with("cross-mirror:"), "mac_history_forbidden");
            if let Some(id) = get::<String>(&self.conn, "route", route)? {
                let r: RequestRecord =
                    get(&self.conn, "request", &id)?.context("unknown_request")?;
                ensure!(
                    can_read(phone, &r)
                        && r.publication.kind != "mirror"
                        && field(p, "project_path") == r.publication.project_path,
                    "source_permission_denied"
                );
                r.source
            } else {
                get::<String>(
                    &self.conn,
                    "timeline",
                    &key((route, field(p, "project_path"))),
                )?
                .context("unknown_timeline")?
            }
        } else {
            phone.windows_source.clone()
        };
        ensure!(
            source == phone.windows_source
                && source_credential(&self.conn, &source)?.client_kind == "windows",
            "source_permission_denied"
        );
        ensure!(self.online(&source)?, "windows_offline");
        let tx = self.conn.transaction()?;
        let epoch = admit(&tx)?;
        let id = job(
            &tx,
            self.budget,
            &source,
            if operation == "open_codex" {
                "open_codex"
            } else if operation == "page" {
                "page"
            } else {
                "sync"
            },
            json!({"operation":operation,"wire":wire,"principal":phone_context(phone),"phone_session":session}),
            epoch,
        )?;
        tx.commit()?;
        Ok(id)
    }

    pub fn rpc_response(&mut self, source: &str, v: Value) -> Result<()> {
        source_credential(&self.conn, source)?;
        ensure!(v.to_string().len() <= MAX_MESSAGE, "message_too_large");
        let id = field(&v, "job_id");
        let tx = self.conn.transaction()?;
        let mut j: Value = get(&tx, "job", id)?.context("unknown_job")?;
        ensure!(
            j["source"] == source && j["epoch"] == v["epoch"] && j["kind"] != "action",
            "wrong_source_job"
        );
        if j["state"] == "completed" {
            ensure!(j["response"] == v["response"], "response_conflict");
            return Ok(());
        }
        ensure!(j["state"] == "dispatched", "job_not_dispatched");
        if j["kind"] == "sync" {
            let events = v["response"]["events"]
                .as_array()
                .context("events_required")?;
            ensure!(events.len() <= 64, "too_many_events");
            for event in events {
                ensure!(
                    matches!(
                        field(event, "message_type"),
                        "mcp_state"
                            | "native_history_sessions"
                            | "timeline_sync_snapshot"
                            | "timeline_sync_delta"
                    ),
                    "unsupported_sync_event"
                );
                // Request publication and mirror state use their own verified path.
                ensure!(
                    !event["payload"]["request"].is_object(),
                    "sync_cannot_publish_request"
                );
                let p = &event["payload"];
                ensure!(
                    field(p, "source") != "cross_device_mirror",
                    "mac_history_forbidden"
                );
                if event["message_type"] == "native_history_sessions" {
                    ensure!(p["source"] == "codex_native", "native_kind_mismatch");
                    for session in p["sessions"]
                        .as_array()
                        .context("invalid_native_sessions")?
                    {
                        let route = field(session, "routeId");
                        let project = field(session, "projectPath");
                        ensure!(
                            route.starts_with("native-cli:") && !project.is_empty(),
                            "invalid_native_session"
                        );
                        let routekey = key((route, project));
                        if let Some(old) = get::<String>(&tx, "timeline", &routekey)? {
                            ensure!(old == source, "timeline_source_conflict");
                        }
                        put(&tx, self.budget, "timeline", &routekey, &source)?;
                    }
                }
                if matches!(
                    field(event, "message_type"),
                    "timeline_sync_snapshot" | "timeline_sync_delta"
                ) {
                    let route = field(p, "timeline_route_id");
                    let project = field(p, "project_path");
                    // Backfill mappings for publications created before the
                    // ordinary-MCP timeline adapter was fixed. Identity still
                    // comes exclusively from the authenticated original request.
                    if get::<String>(&tx, "timeline", &key((route, project)))?.is_none() {
                        if let Some(identity) = get::<String>(&tx, "route", route)? {
                            let request: RequestRecord =
                                get(&tx, "request", &identity)?.context("unknown_request")?;
                            ensure!(
                                request.source == source
                                    && request.publication.kind == "mcp"
                                    && request.publication.project_path == project,
                                "source_permission_denied"
                            );
                            register_timeline(&tx, self.budget, source, &request.publication)?;
                        }
                    }
                    ensure!(
                        get::<String>(&tx, "timeline", &key((route, project)))?.as_deref()
                            == Some(source),
                        "unknown_timeline"
                    );
                    if j["payload"]["operation"] == "timeline" {
                        ensure!(
                            route == field(&j["payload"]["wire"]["payload"], "request_id")
                                && project
                                    == field(&j["payload"]["wire"]["payload"], "project_path"),
                            "timeline_response_mismatch"
                        );
                    }
                }
            }
        }
        j["state"] = json!("completed");
        j["response"] = v["response"].clone();
        put(&tx, self.budget, "job", id, &j)?;
        tx.commit()?;
        Ok(())
    }

    pub fn rpc_result(&self, phone: &Credential, id: &str) -> Result<Option<Value>> {
        let j: Value = get(&self.conn, "job", id)?.context("unknown_job")?;
        ensure!(
            j["payload"]["principal"]["device_id"] == phone.device_id,
            "source_permission_denied"
        );
        Ok(if j["state"] == "completed" {
            Some(j["response"].clone())
        } else {
            None
        })
    }

    pub fn authorize_job(&self, source: &str, id: &str) -> Result<Value> {
        source_credential(&self.conn, source)?;
        let j: Value = get(&self.conn, "job", id)?.context("unknown_job")?;
        let m = meta(&self.conn)?;
        ensure!(
            j["source"] == source
                && j["state"] == "dispatched"
                && j["epoch"] == m["epoch"]
                && (m["frozen"] == false || m["drain"] == true),
            "job_not_authorized"
        );
        let phone: Credential = get(
            &self.conn,
            "phone",
            field(&j["payload"]["principal"], "device_id"),
        )?
        .context("unknown_phone")?;
        ensure!(
            phone.revoked_at.is_none() && phone.client_kind == "android",
            "phone_revoked"
        );
        if j["kind"] == "action" {
            let a: ActionRecord = get(&self.conn, "action", field(&j["payload"], "action_id"))?
                .context("unknown_action")?;
            let latest: RequestRecord = get(
                &self.conn,
                "request",
                &key((
                    &a.source,
                    &a.request.publication.source_request_id,
                    a.request.publication.version,
                )),
            )?
            .context("unknown_request")?;
            ensure!(
                latest.active && can_read(&phone, &latest) && phone.scope("session.respond"),
                "job_not_authorized"
            );
        }
        Ok(json!({"ok":true,"epoch":m["epoch"],"principal":phone_context(&phone)}))
    }

    pub fn image(&mut self, source: &str, v: Value) -> Result<Value> {
        ensure!(
            source_credential(&self.conn, source)?.client_kind == "windows",
            "mirror_text_only"
        );
        let original = field(&v, "original_asset_id");
        let session = field(&v, "source_request_id");
        let version = v["version"].as_u64().context("invalid_version")?;
        ensure!(
            image_id(original) && identifier(session) && version > 0,
            "invalid_image_identity"
        );
        let bytes = STANDARD.decode(field(&v, "data_base64"))?;
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_IMAGE,
            "image_size_limit"
        );
        let mime = field(&v, "mime");
        ensure!(
            matches!(
                mime,
                "image/png" | "image/jpeg" | "image/gif" | "image/webp"
            ),
            "image_mime_denied"
        );
        let identity = key((source, original, session, version));
        let tx = self.conn.transaction()?;
        if let Some(id) = get::<String>(&tx, "asset_map", &identity)? {
            let old: Value = get(&tx, "asset", &id)?.context("missing_asset")?;
            ensure!(
                old["digest"] == hash(&bytes) && old["mime"] == mime,
                "asset_content_conflict"
            );
            return Ok(json!({"id":id,"url":format!("/image?id={id}")}));
        }
        ensure!(v["epoch"].as_u64() == Some(admit(&tx)?), "stale_epoch");
        let id = format!("img_{}", uuid::Uuid::new_v4().simple());
        put(
            &tx,
            self.budget,
            "asset",
            &id,
            &json!({"source":source,"original_asset_id":original,"source_request_id":session,"version":version,"mime":mime,"digest":hash(&bytes),"data_base64":v["data_base64"]}),
        )?;
        put(&tx, self.budget, "asset_map", &identity, &id)?;
        tx.commit()?;
        Ok(json!({"id":id,"url":format!("/image?id={id}")}))
    }
    pub fn read_image(&self, phone: &Credential, id: &str) -> Result<(String, Vec<u8>)> {
        scope(phone, "session.read")?;
        ensure!(image_id(id), "invalid_image_id");
        let v: Value = get(&self.conn, "asset", id)?.context("unknown_asset")?;
        ensure!(
            v["source"] == phone.windows_source
                && source_credential(&self.conn, &phone.windows_source).is_ok(),
            "source_permission_denied"
        );
        ensure!(self.online(&phone.windows_source)?, "windows_offline");
        let identity = key((
            field(&v, "source"),
            field(&v, "source_request_id"),
            v["version"].as_u64().context("invalid_version")?,
        ));
        let r: RequestRecord =
            get(&self.conn, "request", &identity)?.context("asset_not_published")?;
        ensure!(
            can_read(phone, &r)
                && r.publication
                    .wire
                    .to_string()
                    .contains(&format!("/image?id={id}")),
            "asset_session_denied"
        );
        Ok((
            field(&v, "mime").into(),
            STANDARD.decode(field(&v, "data_base64"))?,
        ))
    }
    fn validate_images(&self, source: &str, p: &Publication) -> Result<()> {
        let wire = p.wire.to_string();
        let mut rest = wire.as_str();
        while let Some(i) = rest.find("/image?id=") {
            rest = &rest[i + 10..];
            let id: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            ensure!(image_id(&id), "invalid_image_reference");
            let a: Value = get(&self.conn, "asset", &id)?.context("unknown_asset")?;
            ensure!(
                a["source"] == source
                    && a["source_request_id"] == p.source_request_id
                    && a["version"] == p.version,
                "asset_wrong_session"
            );
        }
        Ok(())
    }

    pub fn control(&mut self, v: Value) -> Result<Value> {
        let tx = self.conn.transaction()?;
        let mut m = meta(&tx)?;
        let previous_frozen = m["frozen"] == true;
        let previous_drain = m["drain"] == true;
        if let Some(frozen) = v["frozen"].as_bool() {
            m["frozen"] = json!(frozen);
        }
        if let Some(drain) = v["drain"].as_bool() {
            m["drain"] = json!(drain);
        }
        if m["frozen"] == false {
            m["drain"] = json!(false);
        }
        // Any relaxation/re-entry, including drain transitions, invalidates
        // acknowledgements from the preceding freeze round at the same epoch.
        if previous_frozen != (m["frozen"] == true) || previous_drain != (m["drain"] == true) {
            m["freeze_generation"] = json!(m["freeze_generation"]
                .as_u64()
                .unwrap_or(0)
                .checked_add(1)
                .context("freeze_generation_exhausted")?);
        }
        if v["advance_epoch"] == true {
            ensure!(m["frozen"] == true && m["drain"] != true, "freeze_required");
            ensure!(all::<Value>(&tx,"desktop_relay")?.iter().all(|job|
                job["state"]=="completed" || job["expires"].as_i64().unwrap_or(0)<now()),
                "unresolved_desktop_requests");
            ensure!(
                all::<Value>(&tx, "job")?
                    .iter()
                    .all(|j| j["state"] == "completed"),
                "unresolved_jobs"
            );
            for c in all::<Credential>(&tx, "source")?
                .into_iter()
                .filter(|c| c.revoked_at.is_none())
            {
                let b: Value =
                    get(&tx, "barrier", &c.device_id)?.context("source_barrier_missing")?;
                let live: Value =
                    get(&tx, "live", &c.device_id)?.context("source_barrier_missing")?;
                ensure!(
                    b["epoch"] == m["epoch"]
                        && b["freeze_generation"].as_u64()
                            == Some(m["freeze_generation"].as_u64().unwrap_or(0))
                        && b["hub_boot"] == self.boot
                        && b["source_generation"].as_u64() == live["generation"].as_u64()
                        && live["boot"] == self.boot
                        && live["at"].as_i64().unwrap_or(0) + 45 >= now()
                        && b["legacy_connections_closed"] == true
                        && b["all_submit_paths_paused"] == true,
                    "source_barrier_missing"
                );
            }
            m["epoch"] = json!(m["epoch"].as_u64().unwrap() + 1);
        }
        put(&tx, self.budget, "meta", "control", &m)?;
        tx.commit()?;
        Ok(m)
    }
    pub fn barrier(&mut self, source: &str, mut v: Value) -> Result<()> {
        source_credential(&self.conn, source)?;
        let tx = self.conn.transaction()?;
        let m = meta(&tx)?;
        let live: Value = get(&tx, "live", source)?.context("source_offline")?;
        ensure!(
            m["frozen"] == true
                && m["drain"] != true
                && v["freeze_generation"].as_u64()
                    == Some(m["freeze_generation"].as_u64().unwrap_or(0))
                && v["source_generation"].as_u64().is_some()
                && v["source_generation"] == live["generation"]
                && live["boot"] == self.boot
                && live["at"].as_i64().unwrap_or(0) + 45 >= now()
                && v["epoch"] == m["epoch"]
                && v["legacy_connections_closed"] == true
                && v["all_submit_paths_paused"] == true,
            "invalid_barrier"
        );
        v["hub_boot"] = json!(self.boot);
        put(&tx, self.budget, "barrier", source, &v)?;
        tx.commit()?;
        Ok(())
    }
    pub fn status(&self) -> Result<Value> {
        let jobs: Vec<Value> = all(&self.conn, "job")?;
        let used: u64 =
            self.conn
                .query_row("SELECT COALESCE(SUM(charge),0) FROM objects", [], |r| {
                    r.get::<_, i64>(0).map(|n| n as u64)
                })?;
        Ok(
            json!({"control":meta(&self.conn)?,"budget":self.budget,"reserved_bytes":used,
            "queued":jobs.iter().filter(|v|v["state"]=="queued").count(),"unknown":jobs.iter().filter(|v|v["state"]=="dispatched").count(),
            "actions":all::<ActionRecord>(&self.conn,"action")?.into_iter().map(|a|json!({"id":a.id,"source":a.source,"state":a.state,"version":a.request.publication.version,"epoch":a.epoch,"winner_record":a.winner_record})).collect::<Vec<_>>()}),
        )
    }
}

fn phone_context(c: &Credential) -> Value {
    json!({"device_id":c.device_id,"client_kind":c.client_kind,"scopes":c.scopes,"file_browser_roots":c.file_browser_roots,"windows_source":c.windows_source,"mirror_sources":c.mirror_sources})
}
fn closed(r: &RequestRecord) -> Value {
    if r.publication.kind == "mirror" {
        json!({"message_type":"cross_device_mirror_state","payload":{"mirror_id":r.publication.route_id,"request_id":r.publication.route_id,"source":"cross_device_mirror","project_path":"","request":null,"closed":true,"origin_device_id":r.source,"origin_platform":"macos"}})
    } else {
        json!({"message_type":"mcp_state","payload":{"request":null,"showMcpPopup":false,"request_id":r.publication.route_id,"project_path":r.publication.project_path,"source":if r.publication.kind=="native"{"codex_native"}else{"mcp"},"origin_device_id":r.source,"origin_platform":"windows"}})
    }
}
fn validate_receipt(a: &ActionRecord, wire: &Value, accepted: bool) -> Result<()> {
    let p = &wire["payload"];
    let action = &a.wire["payload"];
    ensure!(
        p["request_id"] == a.request.publication.route_id
            && field(p, "project_path") == a.request.publication.project_path
            && p["client_action_id"] == action["client_action_id"]
            && p["action"] == action["action"],
        "receipt_identity_mismatch"
    );
    if a.request.publication.kind == "mirror" {
        ensure!(
            field(wire, "message_type") == "cross_device_mirror_action_result"
                && p["mirror_id"] == a.request.publication.route_id
                && p["source"] == "cross_device_mirror",
            "receipt_kind_mismatch"
        );
        if accepted {
            ensure!(
                p["status"] == "accepted"
                    && p["result_kind"]
                        == if action["action"] == "close_mirror" {
                            "mirror_closed"
                        } else {
                            "mac_accepted"
                        },
                "premature_acceptance"
            );
        } else {
            ensure!(p["status"] == "rejected", "invalid_rejection");
        }
    } else {
        ensure!(
            field(wire, "message_type") == "mcp_action_result",
            "receipt_kind_mismatch"
        );
        if accepted {
            ensure!(
                p["status"] == "delivered" && p["delivered"] == true,
                "premature_acceptance"
            );
        } else {
            ensure!(
                p["status"] == "rejected" && p["delivered"] == false,
                "invalid_rejection"
            );
        }
        if a.request.publication.kind == "native" {
            ensure!(
                p["final_result"] == true && p["source"] == "codex_native",
                "native_authority_required"
            );
            if accepted {
                ensure!(
                    matches!(field(p, "result_kind"), "saved" | "cli_received"),
                    "native_result_kind"
                );
            }
        }
    }
    Ok(())
}
