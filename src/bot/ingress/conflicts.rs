//! Content-free, bounded evidence for contradictory deliveries.
use super::*;
use serde_json::Value;

const MAX_NODES: usize = 128;
const MAX_DEPTH: usize = 16;
const MAX_DIFFERENCES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum IngressSource {
    Webhook,
    Polling,
    Gateway,
    Recovery,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum IngressConflictPolicy {
    #[default]
    Stop,
    /// Retain a completed claim and require a matching polling delivery. Never
    /// executes either payload again or resolves a contradictory polling payload.
    RetainCompletedAndPoll,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ConflictDisposition {
    Stop,
    AwaitingPolling,
    PollingConfirmed,
    TimedOut,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PayloadType {
    Missing,
    Null,
    Boolean,
    Number,
    String,
    Array,
    Object,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PayloadDifference {
    /// JSON pointer; unknown object keys are replaced by `*`. No values appear.
    pub path: String,
    pub existing_type: PayloadType,
    pub incoming_type: PayloadType,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct IngressConflict {
    pub bot_id: Uuid,
    pub event_id: u64,
    pub existing_source: IngressSource,
    pub recovered_source: Option<IngressSource>,
    pub incoming_source: IngressSource,
    pub existing_fingerprint: String,
    pub incoming_fingerprint: String,
    pub retained_stage: Option<ReceiptStage>,
    pub disposition: ConflictDisposition,
    pub differences: Vec<PayloadDifference>,
    pub differences_truncated: bool,
}
impl IngressConflict {
    pub fn to_error(&self) -> YappinError {
        YappinError::IngressConflict {
            conflict: Box::new(self.clone()),
        }
    }
    pub(super) fn validate(&self) -> bool {
        let hash = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit());
        hash(&self.existing_fingerprint)
            && hash(&self.incoming_fingerprint)
            && self.differences.len() <= MAX_DIFFERENCES
            && self.differences.iter().all(|d| {
                d.path.len() <= 512
                    && d.path
                        .split('/')
                        .skip(1)
                        .all(|p| p == "*" || p.parse::<usize>().is_ok() || safe_key(p) == p)
            })
    }
}

// Keep neither payload bytes nor arbitrary object keys in the claim ledger.
// Per-claim evidence is bounded independently of the payload size.
#[derive(Clone)]
pub(super) struct Evidence {
    nodes: Vec<Node>,
    truncated: bool,
}
#[derive(Clone)]
struct Node {
    parent: Option<usize>,
    key: Key,
    kind: PayloadType,
    hash: [u8; 32],
}
#[derive(Clone, PartialEq, Eq)]
enum Key {
    Root,
    Field([u8; 32], &'static str),
    Index(usize),
}
fn safe_key(key: &str) -> &'static str {
    match key {
        "id" => "id",
        "type" => "type",
        "data" => "data",
        "created_at" => "created_at",
        "channel_id" => "channel_id",
        "server_id" => "server_id",
        "author_id" => "author_id",
        "user_id" => "user_id",
        "message_id" => "message_id",
        "content" => "content",
        "command" => "command",
        "args" => "args",
        "command_for" => "command_for",
        "author" => "author",
        "author_bot" => "author_bot",
        "mentioned" => "mentioned",
        "reply_to_id" => "reply_to_id",
        "attachment" => "attachment",
        "attachments" => "attachments",
        "components" => "components",
        "embeds" => "embeds",
        "custom_id" => "custom_id",
        "interaction_id" => "interaction_id",
        "token" => "token",
        "expires_at" => "expires_at",
        "is_dm" => "is_dm",
        "dm_channel_id" => "dm_channel_id",
        "username" => "username",
        "emoji" => "emoji",
        "added" => "added",
        "values" => "values",
        "fields" => "fields",
        _ => "*",
    }
}
impl Evidence {
    pub(super) fn new(event: &Event) -> Result<Self> {
        let mut value = serde_json::to_value(event)?;
        canonicalize(&mut value);
        let mut result = Self {
            nodes: vec![],
            truncated: false,
        };
        result.visit(&value, None, Key::Root, 0)?;
        Ok(result)
    }
    fn visit(
        &mut self,
        value: &Value,
        parent: Option<usize>,
        key: Key,
        depth: usize,
    ) -> Result<()> {
        if self.nodes.len() == MAX_NODES || depth > MAX_DEPTH {
            self.truncated = true;
            return Ok(());
        }
        let kind = match value {
            Value::Null => PayloadType::Null,
            Value::Bool(_) => PayloadType::Boolean,
            Value::Number(_) => PayloadType::Number,
            Value::String(_) => PayloadType::String,
            Value::Array(_) => PayloadType::Array,
            Value::Object(_) => PayloadType::Object,
        };
        let index = self.nodes.len();
        self.nodes.push(Node {
            parent,
            key,
            kind,
            hash: Sha256::digest(serde_json::to_vec(value)?).into(),
        });
        match value {
            Value::Object(map) => {
                for (key, value) in map {
                    if self.nodes.len() == MAX_NODES {
                        self.truncated = true;
                        break;
                    }
                    self.visit(
                        value,
                        Some(index),
                        Key::Field(Sha256::digest(key.as_bytes()).into(), safe_key(key)),
                        depth + 1,
                    )?;
                }
            }
            Value::Array(values) => {
                for (i, value) in values.iter().enumerate() {
                    if self.nodes.len() == MAX_NODES {
                        self.truncated = true;
                        break;
                    }
                    self.visit(value, Some(index), Key::Index(i), depth + 1)?;
                }
            }
            _ => (),
        }
        Ok(())
    }
    pub(super) fn differences(&self, other: &Self) -> (Vec<PayloadDifference>, bool) {
        let mut differences = vec![];
        let mut truncated = self.truncated || other.truncated;
        self.compare(0, other, 0, "", &mut differences, &mut truncated);
        (differences, truncated)
    }
    fn compare(
        &self,
        a: usize,
        other: &Self,
        b: usize,
        path: &str,
        out: &mut Vec<PayloadDifference>,
        truncated: &mut bool,
    ) {
        if self.nodes[a].hash == other.nodes[b].hash {
            return;
        }
        if out.len() == MAX_DIFFERENCES {
            *truncated = true;
            return;
        }
        let left = &self.nodes[a];
        let right = &other.nodes[b];
        if left.kind != right.kind || !matches!(left.kind, PayloadType::Object | PayloadType::Array)
        {
            out.push(PayloadDifference {
                path: path.into(),
                existing_type: left.kind,
                incoming_type: right.kind,
            });
            return;
        }
        let before = out.len();
        for (i, node) in self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.parent == Some(a))
        {
            let next = append(path, &node.key);
            if let Some((j, _)) = other
                .nodes
                .iter()
                .enumerate()
                .find(|(_, n)| n.parent == Some(b) && n.key == node.key)
            {
                self.compare(i, other, j, &next, out, truncated);
            } else if out.len() < MAX_DIFFERENCES {
                out.push(PayloadDifference {
                    path: next,
                    existing_type: node.kind,
                    incoming_type: PayloadType::Missing,
                });
            } else {
                *truncated = true;
            }
        }
        for node in other.nodes.iter().filter(|n| n.parent == Some(b)) {
            if !self
                .nodes
                .iter()
                .any(|n| n.parent == Some(a) && n.key == node.key)
            {
                if out.len() < MAX_DIFFERENCES {
                    out.push(PayloadDifference {
                        path: append(path, &node.key),
                        existing_type: PayloadType::Missing,
                        incoming_type: node.kind,
                    });
                } else {
                    *truncated = true;
                }
            }
        }
        if before == out.len() && out.len() < MAX_DIFFERENCES {
            // A differing subtree was outside the evidence budget. Do not imply equality.
            *truncated = true;
            out.push(PayloadDifference {
                path: path.into(),
                existing_type: left.kind,
                incoming_type: right.kind,
            });
        }
    }
}
fn append(path: &str, key: &Key) -> String {
    match key {
        Key::Root => path.into(),
        Key::Field(_, name) => format!("{path}/{name}"),
        Key::Index(i) => format!("{path}/{i}"),
    }
}
pub(super) fn hex(hash: &[u8; 32]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

pub(super) fn canonicalize(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for value in map.values_mut() {
                canonicalize(value);
            }
            map.sort_keys();
        }
        Value::Array(values) => {
            for value in values {
                canonicalize(value);
            }
        }
        _ => (),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn evidence_is_bounded_and_reports_no_values_or_arbitrary_object_keys() {
        let mut event:Event=serde_json::from_value(serde_json::json!({"id":10,"type":"message","data":{"content":"secret-message","credential-as-a-key":"secret-value","author_id":"private-identity"}})).unwrap();
        let evidence = Evidence::new(&event).unwrap();
        event.data["credential-as-a-key"] = serde_json::json!([1]);
        event.data["content"] = serde_json::json!("different-secret");
        event.data["author_id"] = Value::Null;
        let (differences, truncated) = evidence.differences(&Evidence::new(&event).unwrap());
        let json = serde_json::to_string(&differences).unwrap();
        assert!(!truncated);
        assert!(json.contains("/data/*"));
        assert!(json.contains("/data/content"));
        assert!(json.contains("/data/author_id"));
        for value in [
            "secret-message",
            "secret-value",
            "credential-as-a-key",
            "private-identity",
            "different-secret",
        ] {
            assert!(!json.contains(value));
        }
        event.data = serde_json::json!((0..1000).map(|_| "secret-message").collect::<Vec<_>>());
        let evidence = Evidence::new(&event).unwrap();
        assert!(evidence.truncated);
        assert!(evidence.nodes.len() <= MAX_NODES);
    }
    #[test]
    fn object_order_and_outer_timestamp_representation_are_equivalent_but_missing_timestamp_is_not()
    {
        let a:Event=serde_json::from_str(r#"{"id":10,"type":"message","created_at":"2026-11-05T00:00:00Z","data":{"content":"same","author_id":"same"}}"#).unwrap();
        let b:Event=serde_json::from_str(r#"{"data":{"author_id":"same","content":"same"},"created_at":"2026-11-05T01:00:00+01:00","type":"message","id":10}"#).unwrap();
        let ingress = EventIngress::new(IngressConfig::default(), None);
        assert_eq!(
            ingress.fingerprint(&a).unwrap(),
            ingress.fingerprint(&b).unwrap()
        );
        assert!(Evidence::new(&a)
            .unwrap()
            .differences(&Evidence::new(&b).unwrap())
            .0
            .is_empty());
        let mut c = a.clone();
        c.created_at = None;
        c.data["created_at"] = serde_json::json!("2026-11-05T00:00:00Z");
        assert_ne!(
            ingress.fingerprint(&a).unwrap(),
            ingress.fingerprint(&c).unwrap()
        );
        let diff = Evidence::new(&a)
            .unwrap()
            .differences(&Evidence::new(&c).unwrap())
            .0;
        assert!(diff.iter().any(|d| d.path == "/created_at"));
    }
}
