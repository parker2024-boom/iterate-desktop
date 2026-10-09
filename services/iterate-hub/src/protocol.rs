use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const MAX_MESSAGE: usize = 512 * 1024;
pub const MAX_IMAGE: usize = 8 * 1024 * 1024;
pub const MAX_BUDGET: u64 = 1024 * 1024 * 1024;
pub const PUBLIC_URL: &str = "https://8.129.82.226:8443";

pub fn hash(value: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(value)))
}
pub fn token(prefix: &str) -> String {
    format!(
        "{prefix}_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
pub fn field<'a>(v: &'a Value, name: &str) -> &'a str {
    v.get(name).and_then(Value::as_str).unwrap_or("")
}
pub fn identifier(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}
pub fn mirror_id(s: &str) -> bool {
    s.strip_prefix("cross-mirror:").is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
pub fn image_id(s: &str) -> bool {
    s.strip_prefix("img_")
        .is_some_and(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}
pub fn hash_valid(s: &str) -> bool {
    s.strip_prefix("sha256:")
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Credential {
    pub device_id: String,
    pub client_kind: String,
    pub token_hash: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub revoked_at: Option<String>,
    #[serde(default)]
    pub file_browser_roots: Vec<String>,
    #[serde(default)]
    pub windows_source: String,
    #[serde(default)]
    pub mirror_sources: Vec<String>,
}
impl Credential {
    pub fn scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Publication {
    pub source_request_id: String,
    pub version: u64,
    pub registration_key: String,
    pub route_id: String,
    pub kind: String,
    pub project_path: String,
    pub epoch: u64,
    pub published: bool,
    pub wire: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestRecord {
    pub source: String,
    pub publication: Publication,
    pub active: bool,
    #[serde(default)]
    pub winner_action: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionRecord {
    pub id: String,
    pub phone: String,
    pub source: String,
    pub request: RequestRecord,
    pub wire: Value,
    pub state: String,
    pub winner_record: Option<String>,
    pub source_receipt: Option<Value>,
    pub original_response: Option<Value>,
    pub reason: String,
    pub epoch: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Confirmation {
    pub action_id: String,
    pub source_request_id: String,
    pub version: u64,
    pub registration_key: String,
    pub epoch: u64,
    pub state: String,
    #[serde(default)]
    pub durable: bool,
    #[serde(default)]
    pub winner_record: Option<String>,
    #[serde(default)]
    pub receipt: Option<Value>,
    #[serde(default)]
    pub original_response: Option<Value>,
    // A reconciliation proof from the authoritative source ledger, not a timeout.
    #[serde(default)]
    pub definitely_not_executed: bool,
}

pub fn receipt(a: &ActionRecord) -> Value {
    if let Some(v) = &a.source_receipt {
        return v.clone();
    }
    let p = &a.wire["payload"];
    let rejected = a.state == "rejected";
    let mut payload = json!({"request_id": a.request.publication.route_id,
        "project_path": a.request.publication.project_path, "client_action_id": p["client_action_id"],
        "action": p["action"], "status": if rejected {"rejected"} else {"pending"},
        "delivered": false, "reason": a.reason});
    let ty = if a.request.publication.kind == "mirror" {
        payload["mirror_id"] = json!(a.request.publication.route_id);
        payload["source"] = json!("cross_device_mirror");
        payload["result_kind"] = json!(if rejected { "rejected" } else { "pending" });
        "cross_device_mirror_action_result"
    } else {
        if a.request.publication.kind == "native" {
            payload["source"] = json!("codex_native");
            payload["final_result"] = json!(rejected);
        }
        "mcp_action_result"
    };
    json!({"message_type":ty,"payload":payload})
}

pub fn denied(wire: &Value, reason: &str) -> Value {
    let p = &wire["payload"];
    let mirror = field(wire, "message_type") == "cross_device_mirror_action";
    let mut payload = json!({"request_id":p["request_id"],"project_path":p.get("project_path").and_then(Value::as_str).unwrap_or(""),
        "client_action_id":p["client_action_id"],"action":p["action"],"status":"rejected","delivered":false,"reason":reason});
    if mirror {
        payload["mirror_id"] = p["mirror_id"].clone();
        payload["source"] = json!("cross_device_mirror");
        payload["result_kind"] = json!("rejected");
    }
    if field(p, "request_id").starts_with("native-codex:") {
        payload["source"] = json!("codex_native");
        payload["final_result"] = json!(true);
    }
    json!({"message_type":if mirror {"cross_device_mirror_action_result"}else{"mcp_action_result"},"payload":payload})
}

pub fn unknown(wire: &Value) -> Value {
    let mut v = denied(wire, "confirmation_unknown");
    v["payload"]["status"] = json!("pending");
    if v["payload"].get("final_result").is_some() {
        v["payload"]["final_result"] = json!(false);
    }
    if v["payload"].get("result_kind").is_some() {
        v["payload"]["result_kind"] = json!("pending");
    }
    v
}
