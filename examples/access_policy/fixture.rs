//! Synthetic, in-process evidence for the offline demo and tests. Never a live ACL oracle.
use async_trait::async_trait;
use rappin::{Api, Error, Freshness, PermissionSnapshot, Result};
use reqwest::Method;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering},
    Mutex,
};
use uuid::Uuid;

pub fn server() -> Uuid {
    Uuid::from_u128(1)
}
pub fn requester() -> Uuid {
    Uuid::from_u128(2)
}
pub fn bot() -> Uuid {
    Uuid::from_u128(3)
}
pub fn channel() -> Uuid {
    Uuid::from_u128(4)
}
pub fn category() -> Uuid {
    Uuid::from_u128(6)
}

pub struct Fixture {
    pub requester_grant: AtomicBool,
    pub bot_grant: AtomicBool,
    pub administrator: AtomicBool,
    pub owner: Mutex<Uuid>,
    pub channel_row: Mutex<Value>,
    pub categories: Mutex<Value>,
    pub overrides: Mutex<Value>,
    pub override_status: AtomicU16,
    pub member_status: AtomicU16,
    pub writes: AtomicUsize,
    pub freshness: Mutex<Vec<(Uuid, Freshness)>>,
}
impl Default for Fixture {
    fn default() -> Self {
        Self {
            requester_grant: AtomicBool::new(true),
            bot_grant: AtomicBool::new(true),
            administrator: AtomicBool::new(false),
            owner: Mutex::new(Uuid::from_u128(99)),
            channel_row: Mutex::new(
                json!({"id":channel(),"server_id":server(),"kind":"text","category_id":category(),"private":true}),
            ),
            categories: Mutex::new(json!([{"id":category(),"name":"Fixture category"}])),
            overrides: Mutex::new(json!({"overrides":[]})),
            override_status: AtomicU16::new(0),
            member_status: AtomicU16::new(0),
            writes: AtomicUsize::new(0),
            freshness: Mutex::new(vec![]),
        }
    }
}
fn failure(status: u16) -> Error {
    Error::Api {
        status,
        message: "fixture evidence read failed".into(),
        retry_after: None,
    }
}
struct RequestsOnly<'a>(&'a Fixture);
#[async_trait]
impl Api for RequestsOnly<'_> {
    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        self.0.request(method, path, body).await
    }
}
#[async_trait]
impl Api for Fixture {
    async fn permission_snapshot(
        &self,
        server: Uuid,
        actor: Uuid,
        freshness: Freshness,
    ) -> Result<PermissionSnapshot> {
        self.freshness.lock().unwrap().push((actor, freshness));
        // Exercise the public default GET-based implementation, not a prebuilt
        // PermissionEvaluation fixture that could skip freshness/identity checks.
        RequestsOnly(self)
            .permission_snapshot(server, actor, freshness)
            .await
    }
    async fn request(&self, method: Method, path: &str, _body: Option<Value>) -> Result<Value> {
        if method != Method::GET {
            self.writes.fetch_add(1, Ordering::SeqCst);
            return Ok(json!({"ok":true}));
        }
        if path == "/me" {
            return Ok(json!({"id":bot()}));
        }
        if path == format!("/channels/{}", channel()) {
            return Ok(json!({"channel":*self.channel_row.lock().unwrap()}));
        }
        if path == format!("/channels/{}/permissions", channel()) {
            let status = self.override_status.load(Ordering::SeqCst);
            if status != 0 {
                return Err(failure(status));
            }
            return Ok(self.overrides.lock().unwrap().clone());
        }
        if path == format!("/servers/{}", server()) {
            return Ok(
                json!({"server":{"id":server(),"name":"Fixture server","owner_id":*self.owner.lock().unwrap()}}),
            );
        }
        if path == format!("/servers/{}/channels", server()) {
            return Ok(json!({"channels":[],"categories":*self.categories.lock().unwrap()}));
        }
        if path == format!("/servers/{}/roles", server()) {
            return Ok(json!({"roles":[
                {"id":Uuid::from_u128(10),"name":"Requester","permissions":{"manage_channels":self.requester_grant.load(Ordering::SeqCst),"administrator":self.administrator.load(Ordering::SeqCst)}},
                {"id":Uuid::from_u128(11),"name":"Bot","permissions":{"manage_channels":self.bot_grant.load(Ordering::SeqCst)}}
            ]}));
        }
        if path.starts_with(&format!("/servers/{}/members/", server())) {
            let status = self.member_status.load(Ordering::SeqCst);
            if status != 0 {
                return Err(failure(status));
            }
            let actor: Uuid = path.rsplit('/').next().unwrap().parse().unwrap();
            let role = if actor == requester() { 10 } else { 11 };
            return Ok(
                json!({"member":{"id":actor,"username":"fixture","roles":[Uuid::from_u128(role)]}}),
            );
        }
        Err(failure(404))
    }
}
