//! Slot manager for TaskManager — tracks deployed subtasks in slots.

use std::collections::HashMap;

/// State of a single slot on a TaskManager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotState {
    /// Available for deployment.
    Free,
    /// A subtask is running in this slot.
    Occupied { job_id: String, subtask_id: String },
}

/// Manages the execution slots on a TaskManager.
///
/// Each TM has a fixed number of slots. The SlotManager tracks which
/// slots are free vs occupied, handles deploy/cancel requests, and
/// provides slot counts for heartbeat reporting.
pub struct SlotManager {
    slots: Vec<SlotState>,
    /// Map from subtask_id -> slot_index for fast lookup.
    subtask_to_slot: HashMap<String, usize>,
}

impl SlotManager {
    pub fn new(num_slots: usize) -> Self {
        Self {
            slots: vec![SlotState::Free; num_slots],
            subtask_to_slot: HashMap::new(),
        }
    }

    /// Total number of slots.
    pub fn total_slots(&self) -> usize {
        self.slots.len()
    }

    /// Number of slots currently in use.
    pub fn slots_in_use(&self) -> usize {
        self.subtask_to_slot.len()
    }

    /// Number of free slots.
    pub fn slots_available(&self) -> usize {
        self.total_slots() - self.slots_in_use()
    }

    /// Try to deploy a subtask into a free slot.
    /// Returns the slot index on success, or None if no slots available.
    pub fn deploy(&mut self, job_id: &str, subtask_id: &str) -> Option<usize> {
        // Find a free slot.
        let slot_idx = self.slots.iter().position(|s| *s == SlotState::Free)?;

        self.slots[slot_idx] = SlotState::Occupied {
            job_id: job_id.to_string(),
            subtask_id: subtask_id.to_string(),
        };
        self.subtask_to_slot
            .insert(subtask_id.to_string(), slot_idx);

        Some(slot_idx)
    }

    /// Cancel a subtask and free its slot.
    /// Returns true if the subtask was found and removed.
    pub fn cancel(&mut self, subtask_id: &str) -> bool {
        if let Some(slot_idx) = self.subtask_to_slot.remove(subtask_id) {
            self.slots[slot_idx] = SlotState::Free;
            true
        } else {
            false
        }
    }

    /// Cancel all subtasks for a given job.
    /// Returns the number of subtasks cancelled.
    pub fn cancel_job(&mut self, job_id: &str) -> usize {
        let to_remove: Vec<String> = self
            .subtask_to_slot
            .keys()
            .filter(|sid| {
                if let Some(idx) = self.subtask_to_slot.get(*sid) {
                    matches!(&self.slots[*idx], SlotState::Occupied { job_id: jid, .. } if jid == job_id)
                } else {
                    false
                }
            })
            .cloned()
            .collect();

        let count = to_remove.len();
        for subtask_id in to_remove {
            self.cancel(&subtask_id);
        }
        count
    }

    /// Check if a subtask is deployed.
    pub fn is_deployed(&self, subtask_id: &str) -> bool {
        self.subtask_to_slot.contains_key(subtask_id)
    }

    /// Get the slot state at the given index.
    pub fn get_slot(&self, index: usize) -> Option<&SlotState> {
        self.slots.get(index)
    }

    /// List all deployed subtask IDs.
    pub fn deployed_subtasks(&self) -> Vec<String> {
        self.subtask_to_slot.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_slot_manager() {
        let mgr = SlotManager::new(4);
        assert_eq!(mgr.total_slots(), 4);
        assert_eq!(mgr.slots_in_use(), 0);
        assert_eq!(mgr.slots_available(), 4);
    }

    #[test]
    fn test_deploy_and_cancel() {
        let mut mgr = SlotManager::new(2);

        let slot = mgr.deploy("job-1", "map-0");
        assert_eq!(slot, Some(0));
        assert_eq!(mgr.slots_in_use(), 1);
        assert!(mgr.is_deployed("map-0"));

        let slot = mgr.deploy("job-1", "map-1");
        assert_eq!(slot, Some(1));
        assert_eq!(mgr.slots_in_use(), 2);

        // No more slots.
        let slot = mgr.deploy("job-1", "map-2");
        assert_eq!(slot, None);

        // Cancel one.
        assert!(mgr.cancel("map-0"));
        assert_eq!(mgr.slots_available(), 1);
        assert!(!mgr.is_deployed("map-0"));

        // Can deploy again.
        let slot = mgr.deploy("job-2", "reduce-0");
        assert!(slot.is_some());
    }

    #[test]
    fn test_cancel_nonexistent() {
        let mut mgr = SlotManager::new(2);
        assert!(!mgr.cancel("nonexistent"));
    }

    #[test]
    fn test_cancel_job() {
        let mut mgr = SlotManager::new(4);
        mgr.deploy("job-1", "map-0");
        mgr.deploy("job-1", "map-1");
        mgr.deploy("job-2", "reduce-0");

        let cancelled = mgr.cancel_job("job-1");
        assert_eq!(cancelled, 2);
        assert_eq!(mgr.slots_in_use(), 1);
        assert!(mgr.is_deployed("reduce-0"));
        assert!(!mgr.is_deployed("map-0"));
    }

    #[test]
    fn test_deployed_subtasks() {
        let mut mgr = SlotManager::new(4);
        mgr.deploy("job-1", "map-0");
        mgr.deploy("job-1", "map-1");

        let mut subtasks = mgr.deployed_subtasks();
        subtasks.sort();
        assert_eq!(subtasks, vec!["map-0", "map-1"]);
    }

    #[test]
    fn test_slot_states() {
        let mut mgr = SlotManager::new(2);
        assert_eq!(mgr.get_slot(0), Some(&SlotState::Free));

        mgr.deploy("job-1", "map-0");
        assert_eq!(
            mgr.get_slot(0),
            Some(&SlotState::Occupied {
                job_id: "job-1".into(),
                subtask_id: "map-0".into(),
            })
        );
    }
}
