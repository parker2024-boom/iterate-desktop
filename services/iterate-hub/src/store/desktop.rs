use super::*;

fn desktop_peer(db: &Connection, source: &str) -> Result<Value> {
    let own: Value = get(db, "desktop", source)?.context("desktop_not_registered")?;
    ensure!(own["at"].as_i64().unwrap_or(0) + 15 >= now(), "desktop_offline");
    let matches: Vec<Value> = all::<Value>(db, "desktop")?.into_iter().filter(|p| {
        p["source"] != source && p["own_hash"] == own["peer_hash"]
            && p["peer_hash"] == own["own_hash"]
    }).collect();
    ensure!(matches.len() == 1, "desktop_pairing_required");
    let peer = matches.into_iter().next().unwrap();
    source_credential(db, field(&peer, "source"))?;
    ensure!(peer["at"].as_i64().unwrap_or(0) + 15 >= now(), "desktop_peer_offline");
    Ok(peer)
}

impl Store {
    pub fn desktop_announce(&mut self, source: &Credential, v: &Value) -> Result<Value> {
        let own = field(v, "own_hash");
        let peer = field(v, "peer_hash");
        ensure!(hash_valid(own) && hash_valid(peer) && own != peer, "invalid_desktop_pairing");
        let tx = self.conn.transaction()?;
        let value = json!({"source":source.device_id,"own_hash":own,"peer_hash":peer,
            "name":field(v,"name").chars().take(64).collect::<String>(),"enabled":v["enabled"] == true,"at":now()});
        put(&tx, self.budget, "desktop", &source.device_id, &value)?;
        // These transport envelopes are temporary, never original request or
        // winner records. Expiry cannot be interpreted as delivery acceptance.
        tx.execute("DELETE FROM objects WHERE kind='desktop_relay' AND (json_extract(value,'$.expires') < ?1 OR
            (json_extract(value,'$.state')='completed' AND json_extract(value,'$.completed_at') < ?2))", [now(),now()-5])?;
        tx.commit()?;
        self.desktop_status(&source.device_id)
    }

    pub fn desktop_status(&self, source: &str) -> Result<Value> {
        let own: Value = get(&self.conn,"desktop",source)?.context("desktop_not_registered")?;
        match desktop_peer(&self.conn, source) {
            Ok(peer) => Ok(json!({"connected":true,"peer_enabled":peer["enabled"],"peer_name":peer["name"],"enabled":own["enabled"]})),
            Err(e) => Ok(json!({"connected":false,"peer_enabled":false,"enabled":own["enabled"],"error":e.to_string()})),
        }
    }

    pub fn desktop_request(&mut self, source: &str, v: Value) -> Result<Value> {
        let peer = desktop_peer(&self.conn,source)?;
        let path = field(&v,"path");
        ensure!(matches!(path,"/snapshot"|"/sync-windows"|"/submit"|"/api/settings-sync/snapshot"),"desktop_operation_denied");
        let id = field(&v,"id");
        uuid::Uuid::parse_str(id).context("invalid_desktop_request")?;
        ensure!(serde_json::to_vec(&v)?.len() <= MAX_MESSAGE,"message_too_large");
        let tx = self.conn.transaction()?;
        let epoch = admit(&tx)?;
        let binding = hash(serde_json::to_string(&(source,field(&peer,"source"),&v))?.as_bytes());
        if let Some(old) = get::<Value>(&tx,"desktop_relay",id)? {
            ensure!(old["binding"] == binding,"desktop_request_conflict");
        } else {
            put(&tx,self.budget,"desktop_relay",id,&json!({"id":id,"sender":source,"target":peer["source"],
                "binding":binding,"own_hash":peer["own_hash"],"peer_hash":peer["peer_hash"],"epoch":epoch,
                "path":path,"body":v["body"],"expires":now()+30,"state":"pending"}))?;
        }
        tx.commit()?;
        self.desktop_result(source,id)
    }

    pub fn desktop_next(&self, source: &str) -> Result<Value> {
        let peer = desktop_peer(&self.conn,source)?;
        let control = meta(&self.conn)?;
        let jobs: Vec<Value> = all::<Value>(&self.conn,"desktop_relay")?.into_iter().filter(|v| {
            v["target"] == source && v["sender"] == peer["source"] && v["state"] == "pending"
                && v["expires"].as_i64().unwrap_or(0) >= now() && v["epoch"] == control["epoch"]
                && control["frozen"] == false && v["own_hash"] == peer["peer_hash"] && v["peer_hash"] == peer["own_hash"]
        }).collect();
        Ok(json!({"jobs":jobs}))
    }

    pub fn desktop_authorize(&self,source:&str,id:&str)->Result<Value> {
        let next=self.desktop_next(source)?;
        ensure!(next["jobs"].as_array().is_some_and(|jobs|jobs.iter().any(|job|job["id"]==id)),"desktop_request_stale");
        Ok(json!({"ok":true}))
    }

    pub fn desktop_respond(&mut self, source: &str, v: Value) -> Result<Value> {
        let peer = desktop_peer(&self.conn,source)?;
        let tx = self.conn.transaction()?;
        let id = field(&v,"id");
        let mut job: Value = get(&tx,"desktop_relay",id)?.context("desktop_request_unknown")?;
        ensure!(job["target"] == source && job["sender"] == peer["source"]
            && job["own_hash"] == peer["peer_hash"] && job["peer_hash"] == peer["own_hash"],"desktop_response_forbidden");
        ensure!(job["expires"].as_i64().unwrap_or(0) >= now(),"desktop_request_expired");
        ensure!(v["response"].is_object() && serde_json::to_vec(&v)?.len() <= MAX_SOURCE_RESPONSE,"message_too_large");
        if job["state"] == "completed" { ensure!(job["response"] == v["response"],"desktop_response_conflict"); }
        job["response"] = v["response"].clone();
        job["state"] = json!("completed");
        job["completed_at"] = json!(now());
        put(&tx,self.budget,"desktop_relay",id,&job)?;
        tx.commit()?;
        Ok(json!({"ok":true}))
    }

    pub fn desktop_result(&self, source: &str, id: &str) -> Result<Value> {
        let peer = desktop_peer(&self.conn,source)?;
        let job: Value = get(&self.conn,"desktop_relay",id)?.context("desktop_request_unknown")?;
        ensure!(job["sender"] == source && job["target"] == peer["source"],"desktop_result_forbidden");
        ensure!(job["expires"].as_i64().unwrap_or(0) >= now(),"desktop_request_expired");
        Ok(if job["state"] == "completed" {json!({"state":"completed","response":job["response"]})} else {json!({"state":"pending"})})
    }
}
