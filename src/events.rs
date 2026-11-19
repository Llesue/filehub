use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub id: u64,
    pub event: String,
    pub user_id: Option<i64>,
    pub share_id: Option<i64>,
    pub admin_only: bool,
    pub data: Value,
}

impl Event {
    pub fn visible_to_user(&self, user_id: i64, admin: bool) -> bool {
        if self.share_id.is_some() {
            return false;
        }
        if self.admin_only {
            return admin;
        }
        match self.user_id {
            Some(owner) => owner == user_id,
            None => true,
        }
    }

    pub fn visible_to_share(&self, share_id: i64) -> bool {
        self.share_id == Some(share_id)
    }
}

pub struct EventBus {
    tx: broadcast::Sender<Event>,
    next: AtomicU64,
}

impl EventBus {
    pub fn new(capacity: usize) -> Arc<Self> {
        let (tx, _) = broadcast::channel(capacity.max(16));
        Arc::new(Self {
            tx,
            next: AtomicU64::new(1),
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    pub fn publish(&self, event: &str, user_id: Option<i64>, data: Value) {
        self.send(event, user_id, None, false, data);
    }

    pub fn publish_share(&self, event: &str, share_id: i64, data: Value) {
        self.send(event, None, Some(share_id), false, data);
    }

    pub fn publish_admin(&self, event: &str, data: Value) {
        self.send(event, None, None, true, data);
    }

    /// Broadcast to every authenticated user.
    pub fn publish_all(&self, event: &str, data: Value) {
        self.send(event, None, None, false, data);
    }

    fn send(
        &self,
        event: &str,
        user_id: Option<i64>,
        share_id: Option<i64>,
        admin_only: bool,
        data: Value,
    ) {
        let item = Event {
            id: self.next.fetch_add(1, Ordering::Relaxed),
            event: event.to_owned(),
            user_id,
            share_id,
            admin_only,
            data,
        };
        let _ = self.tx.send(item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn user_and_share_scopes_do_not_leak() {
        let bus = EventBus::new(16);
        let mut receiver = bus.subscribe();
        bus.publish("job.updated", Some(1), json!({ "id": "a" }));
        bus.publish_share("share.visited", 9, json!({ "id": 9 }));
        bus.publish_admin("admin.status", json!({ "ok": true }));

        let user_event = receiver.try_recv().unwrap();
        assert!(user_event.visible_to_user(1, false));
        assert!(!user_event.visible_to_user(2, false));
        assert!(!user_event.visible_to_share(9));

        let share_event = receiver.try_recv().unwrap();
        assert!(!share_event.visible_to_user(1, true));
        assert!(share_event.visible_to_share(9));
        assert!(!share_event.visible_to_share(10));

        let admin_event = receiver.try_recv().unwrap();
        assert!(!admin_event.visible_to_user(1, false));
        assert!(admin_event.visible_to_user(1, true));
        assert!(!admin_event.visible_to_share(1));
    }

    #[test]
    fn ids_are_unique_and_increasing() {
        let bus = EventBus::new(16);
        let mut receiver = bus.subscribe();
        bus.publish("a", Some(1), json!({}));
        bus.publish_share("b", 2, json!({}));
        bus.publish_admin("c", json!({}));
        let first = receiver.try_recv().unwrap().id;
        let second = receiver.try_recv().unwrap().id;
        let third = receiver.try_recv().unwrap().id;
        assert_eq!(second, first + 1);
        assert_eq!(third, second + 1);
    }
}

#[cfg(test)]
mod broadcast_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn broadcast_events_are_visible_to_every_user() {
        let bus = EventBus::new(16);
        let mut receiver = bus.subscribe();
        bus.publish_all("settings.changed", json!({ "keys": ["ui.brand_name"] }));
        let event = receiver.try_recv().unwrap();
        assert!(event.visible_to_user(1, false));
        assert!(event.visible_to_user(2, true));
        assert!(event.visible_to_user(3, false));
        assert!(!event.visible_to_share(1));
    }

    #[test]
    fn admin_only_events_stay_admin_only() {
        let bus = EventBus::new(16);
        let mut receiver = bus.subscribe();
        bus.publish_admin("user.updated", json!({ "id": 1 }));
        let event = receiver.try_recv().unwrap();
        assert!(event.visible_to_user(1, true));
        assert!(!event.visible_to_user(1, false));
    }
}
