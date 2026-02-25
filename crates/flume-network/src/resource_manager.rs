//! Resource Manager — tracks registered TaskManagers and their slot capacity.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use flume_runtime::physical::SlotOffer;

use crate::proto::jobmanager::{HeartbeatRequest, RegisterRequest};

/// State of a registered TaskManager.
#[derive(Debug, Clone)]
pub struct TaskManagerInfo {
    pub id: String,
    pub address: String,
    pub num_slots: u32,
    pub slots_in_use: u32,
    pub registered_jobs: Vec<String>,
    pub last_heartbeat: Instant,
}

impl TaskManagerInfo {
    pub fn slots_available(&self) -> u32 {
        self.num_slots - self.slots_in_use
    }
}

/// Manages the inventory of TaskManagers.
pub struct ResourceManager {
    task_managers: HashMap<String, TaskManagerInfo>,
    heartbeat_timeout: Duration,
}

impl ResourceManager {
    pub fn new(heartbeat_timeout: Duration) -> Self {
        Self {
            task_managers: HashMap::new(),
            heartbeat_timeout,
        }
    }

    /// Register a new TM. Returns true if accepted.
    pub fn register(&mut self, req: &RegisterRequest) -> bool {
        let info = TaskManagerInfo {
            id: req.task_manager_id.clone(),
            address: req.address.clone(),
            num_slots: req.num_slots,
            slots_in_use: 0,
            registered_jobs: req.registered_jobs.clone(),
            last_heartbeat: Instant::now(),
        };
        self.task_managers.insert(req.task_manager_id.clone(), info);
        true
    }

    /// Update heartbeat timestamp and slot usage. Returns false if TM is unknown.
    pub fn heartbeat(&mut self, req: &HeartbeatRequest) -> bool {
        if let Some(info) = self.task_managers.get_mut(&req.task_manager_id) {
            info.last_heartbeat = Instant::now();
            info.slots_in_use = req.slots_in_use;
            true
        } else {
            false
        }
    }

    /// Return IDs of TMs whose last heartbeat is older than the timeout.
    pub fn detect_dead(&self) -> Vec<String> {
        let now = Instant::now();
        self.task_managers
            .values()
            .filter(|info| now.duration_since(info.last_heartbeat) > self.heartbeat_timeout)
            .map(|info| info.id.clone())
            .collect()
    }

    /// Remove a dead TM from the registry.
    pub fn remove(&mut self, tm_id: &str) -> Option<TaskManagerInfo> {
        self.task_managers.remove(tm_id)
    }

    /// Return all active TMs as SlotOffers for scheduling.
    pub fn slot_offers(&self) -> Vec<SlotOffer> {
        self.task_managers
            .values()
            .map(|info| SlotOffer {
                task_manager_id: info.id.clone(),
                total_slots: info.num_slots as usize,
                used_slots: info.slots_in_use as usize,
            })
            .collect()
    }

    /// Get info for a specific TM.
    pub fn get(&self, tm_id: &str) -> Option<&TaskManagerInfo> {
        self.task_managers.get(tm_id)
    }

    /// List all registered TMs.
    pub fn list(&self) -> Vec<&TaskManagerInfo> {
        self.task_managers.values().collect()
    }

    /// Update slot usage for a TM.
    pub fn update_slot_usage(&mut self, tm_id: &str, slots_in_use: u32) {
        if let Some(info) = self.task_managers.get_mut(tm_id) {
            info.slots_in_use = slots_in_use;
        }
    }

    /// Number of registered TMs.
    pub fn len(&self) -> usize {
        self.task_managers.len()
    }

    /// Returns true if no TMs are registered.
    pub fn is_empty(&self) -> bool {
        self.task_managers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_register_request(id: &str, slots: u32) -> RegisterRequest {
        RegisterRequest {
            task_manager_id: id.into(),
            address: format!("127.0.0.1:{}", 50000 + slots),
            num_slots: slots,
            registered_jobs: vec!["job-a".into()],
        }
    }

    #[test]
    fn test_register_and_list() {
        let mut rm = ResourceManager::new(Duration::from_secs(30));
        assert!(rm.register(&make_register_request("tm-1", 4)));
        assert!(rm.register(&make_register_request("tm-2", 2)));
        assert!(rm.register(&make_register_request("tm-3", 8)));

        assert_eq!(rm.len(), 3);
        assert_eq!(rm.list().len(), 3);
        assert!(rm.get("tm-1").is_some());
        assert!(rm.get("tm-99").is_none());
    }

    #[test]
    fn test_heartbeat_updates() {
        let mut rm = ResourceManager::new(Duration::from_secs(30));
        rm.register(&make_register_request("tm-1", 4));

        let hb = HeartbeatRequest {
            task_manager_id: "tm-1".into(),
            slots_available: 2,
            slots_in_use: 2,
        };
        assert!(rm.heartbeat(&hb));
        assert_eq!(rm.get("tm-1").unwrap().slots_in_use, 2);

        // Unknown TM returns false.
        let hb_unknown = HeartbeatRequest {
            task_manager_id: "tm-99".into(),
            slots_available: 0,
            slots_in_use: 0,
        };
        assert!(!rm.heartbeat(&hb_unknown));
    }

    #[test]
    fn test_detect_dead() {
        let mut rm = ResourceManager::new(Duration::from_millis(10));
        rm.register(&make_register_request("tm-1", 4));

        // Immediately after registration, should not be dead.
        assert!(rm.detect_dead().is_empty());

        // Sleep past the timeout.
        std::thread::sleep(Duration::from_millis(20));
        let dead = rm.detect_dead();
        assert_eq!(dead, vec!["tm-1"]);
    }

    #[test]
    fn test_remove() {
        let mut rm = ResourceManager::new(Duration::from_secs(30));
        rm.register(&make_register_request("tm-1", 4));

        let removed = rm.remove("tm-1");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().id, "tm-1");
        assert!(rm.is_empty());
    }

    #[test]
    fn test_slot_offers() {
        let mut rm = ResourceManager::new(Duration::from_secs(30));
        rm.register(&make_register_request("tm-1", 4));
        rm.update_slot_usage("tm-1", 1);

        let offers = rm.slot_offers();
        assert_eq!(offers.len(), 1);
        assert_eq!(offers[0].total_slots, 4);
        assert_eq!(offers[0].used_slots, 1);
        assert_eq!(offers[0].available(), 3);
    }
}
