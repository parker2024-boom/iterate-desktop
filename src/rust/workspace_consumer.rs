//! Optional loopback presentation. The original request, arbitration and MCP own delivery.
use serde_json::{json, Value};
use std::{path::Path, sync::{Arc, Mutex}, time::{Duration, Instant}};
use crate::{cross_device::Registration, delivery::{Delivery, ResponseFileClaim, publish_claimed_response_file}};

pub(crate) struct Consumer {
    client: reqwest::Client,
    endpoint: String,
    id: String,
    token: Mutex<String>,
    request: Value,
    source_key: String,
    project: String,
}

impl Consumer {
    async fn call(&self, action: &str, extra: Value) -> Result<Value, String> {
        let token=self.token.lock().map_err(|_|"Consumer token lock poisoned")?.clone();
        let mut body = json!({"request_id":self.id,"token":token});
        if let Some(fields) = extra.as_object() {
            body.as_object_mut().unwrap().extend(fields.clone());
        }
        let response = self.client.post(format!("{}/api/iterate/{}", self.endpoint, action))
            .json(&body).send().await.map_err(|e|e.to_string())?;
        response.error_for_status().map_err(|e|e.to_string())?
            .json().await.map_err(|e|e.to_string())
    }

    async fn restore(&self) -> Result<(), String> {
        let response=self.client.post(format!("{}/api/iterate/offer",self.endpoint))
            .json(&json!({"request":self.request,"source_key":self.source_key,"phase":"prepare"})).send().await.map_err(|e|e.to_string())?
            .error_for_status().map_err(|e|e.to_string())?;
        let state:Value=response.json().await.map_err(|e|e.to_string())?;
        let token=state.get("token").and_then(Value::as_str).ok_or("Consumer registration missing token")?;
        *self.token.lock().map_err(|_|"Consumer token lock poisoned")?=token.to_string();
        Ok(())
    }

    async fn call_recovering(&self, action:&str, extra:Value) -> Result<Value,String> {
        match self.call(action,extra.clone()).await {
            Ok(value)=>Ok(value),
            Err(_)=>{self.restore().await?;self.call("commit",json!({})).await?;self.call(action,extra).await}
        }
    }

    pub(crate) async fn offer(request: &Value) -> Option<Self> {
        let diagnose=|reason: &str|crate::app::cli::instance_debug_log("[workspace-consumer-offer]",format!("request_id={:?}, thread_id={:?}, provenance={:?}, result={}",request.get("id"),request.get("codex_thread_id"),request.get("codex_thread_provenance"),reason));
        if request.get("codex_thread_provenance").and_then(Value::as_str) != Some("caller_meta") { diagnose("untrusted_caller");return None; }
        let endpoint = std::env::var("ITERATE_WORKSPACE_URL").unwrap_or_else(|_|"http://127.0.0.1:8799".into());
        // This presentation channel is never a remote submission or a proxy route.
        let url = reqwest::Url::parse(&endpoint).ok()?;
        if url.scheme() != "http" || url.host_str() != Some("127.0.0.1") || !url.username().is_empty() || url.password().is_some() { return None; }
        let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(2)).build().ok()?;
        // Do not send a prepare to an older broker that could expose it immediately.
        let capabilities:Value=client.get(format!("{}/api/iterate/protocol",endpoint.trim_end_matches('/')))
            .send().await.ok()?.error_for_status().ok()?.json().await.ok()?;
        if capabilities.get("prepare_commit").and_then(Value::as_bool)!=Some(true){return None;}
        let source_key=uuid::Uuid::new_v4().to_string();
        let response = match client.post(format!("{}/api/iterate/offer",endpoint.trim_end_matches('/')))
            .json(&json!({"request":request,"source_key":source_key,"phase":"prepare"})).send().await {
                Ok(response)=>response,
                Err(error)=>{diagnose(if error.is_timeout(){"http_timeout"}else{"http_connection_failed"});return None;}
            };
        let status=response.status();
        let response: Value=match response.json().await {Ok(value)=>value,Err(_)=>{diagnose("invalid_consumer_json");return None;}};
        if !status.is_success(){diagnose(&format!("http_{}: {}",status.as_u16(),response.get("error").and_then(Value::as_str).unwrap_or("rejected")));return None;}
        diagnose("accepted");
        Some(Self { client, endpoint:endpoint.trim_end_matches('/').into(), id:request.get("id")?.as_str()?.into(), token:Mutex::new(response.get("token")?.as_str()?.into()), request:request.clone(), source_key, project:request.get("project_path")?.as_str()?.into() })
    }

    pub(crate) async fn wait<F: FnOnce()>(&self, registration: Option<&Registration>, response_file: &Path, delivery: Option<&Arc<Delivery>>, request: &Value, ready_file: &Path, on_ready: F) -> Result<(), String> {
        // The accepted offer selected this route once. Browser visibility is not ownership.
        let _presentation=Presentation::register(request,ready_file)?;
        on_ready();
        // Receiving the prepare token selected workspace locally. An uncertain commit
        // must retry this route, while a lost prepare response remains unanswerable.
        let _=self.call_recovering("commit",json!({})).await;
        loop {
            if delivery.is_some_and(|d|d.disconnected()) {
                let _ = self.call_recovering("finish",json!({"status":"failed","error":crate::delivery::FAILURE})).await;
                return Err(crate::delivery::FAILURE.into());
            }
            if let Some(reg) = registration {
                reg.renew();
                // Only an existing winning receipt is materialized, never a cancellation.
                let _ = reg.materialize_consumer_winner();
            }
            if response_file.exists() { return Ok(()); }
            if let Ok(state) = self.call_recovering("poll",json!({})).await {
                    if state.get("committed").and_then(Value::as_bool)==Some(false) {
                        // A prior commit may never have reached the broker. A
                        // successful poll of a prepared entry is not acceptance.
                        let _=self.call_recovering("commit",json!({})).await;
                    } else if let Some(proposal) = state.get("proposal").filter(|p|p.is_object()) {
                        let result = if let Some(reg) = registration {
                            reg.submit_consumer(proposal).and_then(|_|reg.materialize_consumer_winner().map(|_|()))
                        } else {
                            submit_unregistered(response_file,proposal)
                        };
                        if let Err(error) = result {
                            // A peer may already own the winner; observe it on the next iteration.
                            let _ = self.call("reject",json!({"error":error})).await;
                        }
                    }
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }

    pub(crate) fn track_handoff(self, delivery: Option<Arc<Delivery>>, response: &Value) {
        let response=response.clone();
        let source = response.get("metadata").and_then(|m|m.get("source")).cloned().unwrap_or(Value::Null);
        tokio::spawn(async move {
            let started=Instant::now();
            loop {
                let status = match delivery.as_ref() {
                    Some(d) if d.was_returned()=>Some("completed"),
                    Some(d) if d.disconnected()=>Some("failed"),
                    None=>Some("completed"),
                    _=>None,
                };
                if let Some(status)=status {
                    // Retry a lost callback; Python idempotently retains terminal status.
                    for _ in 0..120 {
                        if self.call_recovering("finish",json!({"status":status,"source":source,"response":response})).await.is_ok(){break;}
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                    if status=="completed" {
                        let url=format!("{}/bridge/publish",crate::app::cli::local_bridge_base_url());
                        let message=json!({"message_type":"mcp_state","payload":{"request":null,"showMcpPopup":false,"request_id":self.id,"project_path":self.project}});
                        if let Ok(request)=crate::bridge::auth::authorize_internal_bridge_request(self.client.post(&url).json(&message),"POST",&url) {
                            let _=request.send().await;
                        }
                    }
                    break;
                }
                if started.elapsed()>Duration::from_secs(30) {
                    let _=self.call_recovering("finish",json!({"status":"failed","error":"回复交付尚未确认"})).await;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });
    }
}

struct Presentation { ready_file: std::path::PathBuf }
impl Presentation {
    fn register(request: &Value, ready_file: &Path) -> Result<Self,String> {
        let id=request.get("id").and_then(Value::as_str).ok_or("Missing request ID")?;
        let project=request.get("project_path").and_then(Value::as_str).ok_or("Missing project")?;
        let mut registry=crate::ui::window_registry::WindowRegistry::load();
        registry.register(project,Some(id),request.get("conversation_title").and_then(Value::as_str))?;
        let guard=Self {ready_file:ready_file.into()};
        std::fs::write(ready_file,json!({"request_id":id,"project_path":project,"ready_at":chrono::Utc::now().to_rfc3339()}).to_string()).map_err(|e|e.to_string())?;
        Ok(guard)
    }
}
impl Drop for Presentation {
    fn drop(&mut self) {
        let _=crate::ui::window_registry::WindowRegistry::load().unregister();
        let _=std::fs::remove_file(&self.ready_file);
    }
}

fn submit_unregistered(path: &Path, response: &Value) -> Result<(), String> {
    let _admission=crate::cross_device::hub_transport::submission_guard(false)?;
    let mut claim=ResponseFileClaim::acquire(path).map_err(|e|e.to_string())?;
    publish_claimed_response_file(path,&response.to_string()).map_err(|e|e.to_string())?;
    claim.commit();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unregistered_consumer_cannot_overwrite_original_response() {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("response.json");
        std::fs::write(&path,"{\"user_input\":\"peer winner\"}").unwrap();
        assert!(submit_unregistered(&path,&json!({"user_input":"local loser"})).is_err());
        assert!(std::fs::read_to_string(&path).unwrap().contains("peer winner"));
    }
}
