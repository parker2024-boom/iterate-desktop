//! Desktop peers use the public Hub and their existing mutual pairing identity.
//! No device address or inbound listener participates in this transport.
use super::{atomic_json, directory, hub_transport, read_json, transport, Result};
use serde_json::{json, Value};
use std::{collections::HashMap, time::Duration};

pub(super) async fn status() -> Result<Value> {
    if let Ok(cached)=read_json::<Value>(&directory()?.join("desktop-hub-status.json")) {
        if cached["at"].as_i64().is_some_and(|at|at+15>=chrono::Utc::now().timestamp())
            && cached["revision"].as_str()==transport::route_key().ok().as_deref() {
            return Ok(cached["status"].clone());
        }
    }
    hub_transport::desktop_call("/source/desktop-status",&json!({})).await
}

pub(super) fn snapshot_cached()->Option<Value> {
    let cached:Value=read_json(&directory().ok()?.join("desktop-hub-snapshot.json")).ok()?;
    (cached["at"].as_i64().is_some_and(|at|at+10>=chrono::Utc::now().timestamp())
        && cached["revision"].as_str()==transport::route_key().ok().as_deref()).then(||cached["snapshot"].clone())
}

pub(super) async fn request(path:&str,body:Option<&Value>)->Result<Value> {
    let id=uuid::Uuid::new_v4().to_string();
    let revision=transport::route_key()?;
    let payload=json!({"id":id,"path":path,"body":body});
    let mut result=hub_transport::desktop_call("/source/desktop-request",&payload).await?;
    let deadline=tokio::time::Instant::now()+Duration::from_secs(10);
    loop {
        if transport::route_key()?!=revision {return Err("公网配对已改变，请重新连接".into());}
        if result["state"]=="completed" {
            let response=&result["response"];
            if response["status"]!=200 {return Err(format!("公网对端请求失败：{}",response["status"]));}
            return Ok(response["body"].clone());
        }
        if tokio::time::Instant::now()>=deadline {return Err("公网对端未确认响应，请检查对端是否在线".into());}
        tokio::time::sleep(Duration::from_millis(100)).await;
        result=hub_transport::desktop_call("/source/desktop-result",&json!({"id":id})).await?;
    }
}

pub(super) async fn relay_loop() {
    // Keep completed RPC replies through a lost Hub acknowledgement. Actual
    // submissions still pass through the original durable request/winner lock.
    let mut receipts:HashMap<String,(i64,Value)>=HashMap::new();
    loop {
        if let Ok(identity)=transport::cloud_identity() {
            if let Ok(status)=hub_transport::desktop_call("/source/desktop-announce",&identity).await {
                if let Ok(dir)=directory() {
                    let _=atomic_json(&dir.join("desktop-hub-status.json"),&json!({"at":chrono::Utc::now().timestamp(),
                        "revision":transport::route_key().ok(),"status":status}));
                }
                if let Ok(next)=hub_transport::desktop_call("/source/desktop-next",&json!({})).await {
                    for job in next["jobs"].as_array().into_iter().flatten() {
                        let Some(id)=job["id"].as_str() else {continue};
                        // The Hub checks reciprocal pair hashes. Repeat the
                        // check against current local credentials before use.
                        if job["own_hash"]!=identity["own_hash"]||job["peer_hash"]!=identity["peer_hash"] {continue;}
                        if job["expires"].as_i64().unwrap_or(0)<chrono::Utc::now().timestamp() {continue;}
                        let response=if let Some((_,response))=receipts.get(id){response.clone()}else{
                            if hub_transport::desktop_call("/source/desktop-authorize",&json!({"id":id})).await.is_err(){continue;}
                            let response=super::cloud_desktop_dispatch(job["path"].as_str().unwrap_or(""),job["body"].clone()).await;
                            receipts.insert(id.into(),(chrono::Utc::now().timestamp(),response.clone()));
                            response
                        };
                        let _=hub_transport::desktop_call("/source/desktop-respond",&json!({"id":id,"response":response})).await;
                    }
                }
            }
        }
        let now=chrono::Utc::now().timestamp();
        receipts.retain(|_,(at,_)|*at+90>=now);
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

pub(super) fn daemon_ready()->bool {
    directory().ok().and_then(|d|read_json::<Value>(&d.join("desktop-hub-health.json")).ok())
        .is_some_and(|v|v["at"].as_i64().is_some_and(|at|at+15>=chrono::Utc::now().timestamp())
            && v["revision"].as_str()==transport::route_key().ok().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn public_request_uses_hub_without_resolving_device_addresses() {
        use axum::{extract::State,routing::post,Json};
        use std::sync::{Arc,Mutex};
        struct Restore(Vec<(&'static str,Option<std::ffi::OsString>)>);
        impl Drop for Restore {fn drop(&mut self){for (key,value) in &self.0 {match value {
            Some(value)=>std::env::set_var(key,value),None=>std::env::remove_var(key),
        }}}}
        let _restore=Restore(["ITERATE_CROSS_DEVICE_DIR","TEST_PUBLIC_HUB_TOKEN","ITERATE_CROSS_DEVICE_MIRROR","ITERATE_CROSS_DEVICE_SOURCE"].into_iter()
            .map(|key|(key,std::env::var_os(key))).collect());
        let dir=tempfile::tempdir().unwrap();
        std::env::set_var("ITERATE_CROSS_DEVICE_DIR",dir.path());
        std::env::set_var("TEST_PUBLIC_HUB_TOKEN","synthetic-public-fixture-token");
        let observed=Arc::new(Mutex::new(Vec::<Value>::new()));
        let app=axum::Router::new()
            .route("/source/desktop-request",post(|State(observed):State<Arc<Mutex<Vec<Value>>>>,Json(body):Json<Value>|async move {
                observed.lock().unwrap().push(body);Json(json!({"state":"pending"}))
            }))
            .route("/source/desktop-result",post(||async {Json(json!({"state":"completed","response":{"status":200,"body":{"device_name":"peer","requests":[],"ok":true}}}))}))
            .with_state(observed.clone());
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint=format!("http://{}",listener.local_addr().unwrap());
        super::super::atomic_json(&dir.path().join("hub-connection.json"),&hub_transport::HubConfig{
            endpoint,device_id:"public-fixture".into(),token_env:"TEST_PUBLIC_HUB_TOKEN".into(),ca_certificate:None}).unwrap();
        // Neither address belongs to this machine, and no port is listening.
        super::super::atomic_json(&dir.path().join("direct-connection.json"),&json!({
            "device_name":"fixture","listen_ip":"203.0.113.30","peer_host":"203.0.113.40","listen_port":5540,"peer_port":5540})).unwrap();
        let server=tokio::spawn(async move {axum::serve(listener,app).await.unwrap()});
        let result=transport::peer("/snapshot",None).await.unwrap();
        assert_eq!(result["device_name"],"peer");
        let key="56a507d5-c508-4e2c-9108-7e65eb64ae10";
        super::super::atomic_json(&dir.path().join("hub-control.json"),&json!({"frozen":false,"drain":false,"epoch":1})).unwrap();
        std::env::set_var("ITERATE_CROSS_DEVICE_MIRROR",key);
        std::env::remove_var("ITERATE_CROSS_DEVICE_SOURCE");
        assert!(super::super::submit(&json!("CANCELLED")).await.unwrap());
        assert!(!dir.path().join("dismissed").join(key).exists());
        let calls=observed.lock().unwrap();
        assert_eq!(calls.len(),2);
        assert_eq!(calls[0]["path"],"/snapshot");
        assert!(calls[0]["body"].is_null());
        assert_eq!(calls[1]["path"],"/submit");
        assert_eq!(calls[1]["body"]["key"],key);
        assert_eq!(calls[1]["body"]["response"]["metadata"]["source"],"popup_closed");
        server.abort();
    }
}
