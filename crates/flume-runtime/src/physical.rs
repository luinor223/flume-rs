//! Physical execution graph — maps logical operators to concrete subtask instances
//! assigned to TaskManager slots.

use serde::{Deserialize, Serialize};

use crate::dag::{LogicalGraph, NodeId, PartitionStrategy};

/// Unique identifier for a TaskManager.
pub type TaskManagerId = String;

/// Unique identifier for a subtask instance (one parallel instance of an operator).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubtaskId {
    pub node_id: NodeId,
    pub operator_name: String,
    pub subtask_index: usize,
}

impl SubtaskId {
    pub fn new(node_id: NodeId, operator_name: impl Into<String>, subtask_index: usize) -> Self {
        Self {
            node_id,
            operator_name: operator_name.into(),
            subtask_index,
        }
    }

    /// Canonical string form: `"{operator_name}-{subtask_index}"`.
    pub fn canonical_name(&self) -> String {
        format!("{}-{}", self.operator_name, self.subtask_index)
    }
}

impl std::fmt::Display for SubtaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.canonical_name())
    }
}

/// Assignment of a subtask to a TaskManager slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtaskAssignment {
    pub subtask_id: SubtaskId,
    pub task_manager_id: TaskManagerId,
    pub slot_index: usize,
}

/// A physical edge connecting two specific subtask instances.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhysicalEdge {
    pub from: SubtaskId,
    pub to: SubtaskId,
    pub partition_strategy: PartitionStrategy,
}

/// The physical execution graph: all subtask instances with their TM assignments
/// and the concrete data edges between them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhysicalGraph {
    pub assignments: Vec<SubtaskAssignment>,
    pub edges: Vec<PhysicalEdge>,
}

/// Scheduling strategy for assigning subtasks to TM slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulingStrategy {
    /// Spread subtasks evenly across TMs for fault isolation.
    Spread,
    /// Pack subtasks onto fewer TMs for co-location.
    Pack,
}

/// A slot offer from a TaskManager.
#[derive(Debug, Clone)]
pub struct SlotOffer {
    pub task_manager_id: TaskManagerId,
    pub total_slots: usize,
    pub used_slots: usize,
}

impl SlotOffer {
    pub fn available(&self) -> usize {
        self.total_slots - self.used_slots
    }
}

/// Convert a `LogicalGraph` into a `PhysicalGraph` by assigning subtask instances
/// to available TaskManager slots.
pub fn schedule(
    graph: &LogicalGraph,
    slots: &[SlotOffer],
    strategy: SchedulingStrategy,
) -> Result<PhysicalGraph, String> {
    let topo = graph
        .topological_order()
        .ok_or("logical graph contains a cycle")?;

    // Count total subtasks needed.
    let total_subtasks: usize = graph.nodes.iter().map(|n| n.parallelism).sum();
    let total_available: usize = slots.iter().map(|s| s.available()).sum();
    if total_subtasks > total_available {
        return Err(format!(
            "not enough slots: need {total_subtasks}, have {total_available}"
        ));
    }

    // Build slot assignment order based on strategy.
    let mut slot_queue = build_slot_queue(slots, strategy);
    let mut assignments = Vec::with_capacity(total_subtasks);

    // Assign subtasks in topological order.
    for &node_id in &topo {
        let node = &graph.nodes[node_id];
        for subtask_index in 0..node.parallelism {
            let (tm_id, slot_idx) = slot_queue
                .pop()
                .ok_or("ran out of slots during assignment")?;
            assignments.push(SubtaskAssignment {
                subtask_id: SubtaskId::new(node_id, &node.name, subtask_index),
                task_manager_id: tm_id,
                slot_index: slot_idx,
            });
        }
    }

    // Generate physical edges.
    let edges = generate_edges(graph, &assignments);

    Ok(PhysicalGraph { assignments, edges })
}

/// Build a queue of (task_manager_id, slot_index) pairs ordered by strategy.
fn build_slot_queue(
    slots: &[SlotOffer],
    strategy: SchedulingStrategy,
) -> Vec<(TaskManagerId, usize)> {
    let mut offers: Vec<&SlotOffer> = slots.iter().collect();
    match strategy {
        SchedulingStrategy::Spread => {
            // Round-robin: interleave slots from different TMs.
            // Sort by most available first so spread is even.
            offers.sort_by_key(|o| std::cmp::Reverse(o.available()));
            let mut queues: Vec<Vec<(TaskManagerId, usize)>> = offers
                .iter()
                .map(|o| {
                    (o.used_slots..o.total_slots)
                        .map(|i| (o.task_manager_id.clone(), i))
                        .collect()
                })
                .collect();

            let mut result = Vec::new();
            let mut any = true;
            while any {
                any = false;
                for q in &mut queues {
                    if let Some(slot) = q.first().cloned() {
                        q.remove(0);
                        result.push(slot);
                        any = true;
                    }
                }
            }
            result
        }
        SchedulingStrategy::Pack => {
            // Fill TMs in order (least available first = most packed).
            offers.sort_by_key(|o| o.available());
            offers
                .iter()
                .flat_map(|o| (o.used_slots..o.total_slots).map(|i| (o.task_manager_id.clone(), i)))
                .collect()
        }
    }
}

/// Generate physical edges from logical edges based on partition strategy.
fn generate_edges(graph: &LogicalGraph, assignments: &[SubtaskAssignment]) -> Vec<PhysicalEdge> {
    let mut edges = Vec::new();

    for logical_edge in &graph.edges {
        let _from_node = &graph.nodes[logical_edge.from];
        let _to_node = &graph.nodes[logical_edge.to];

        let from_subtasks: Vec<&SubtaskAssignment> = assignments
            .iter()
            .filter(|a| a.subtask_id.node_id == logical_edge.from)
            .collect();
        let to_subtasks: Vec<&SubtaskAssignment> = assignments
            .iter()
            .filter(|a| a.subtask_id.node_id == logical_edge.to)
            .collect();

        match logical_edge.partition_strategy {
            PartitionStrategy::Forward => {
                // 1:1 mapping by index (modulo if parallelism differs).
                for (i, from) in from_subtasks.iter().enumerate() {
                    let target_idx = i % to_subtasks.len();
                    edges.push(PhysicalEdge {
                        from: from.subtask_id.clone(),
                        to: to_subtasks[target_idx].subtask_id.clone(),
                        partition_strategy: PartitionStrategy::Forward,
                    });
                }
            }
            PartitionStrategy::Hash | PartitionStrategy::Rebalance => {
                // All-to-all: every upstream subtask connects to every downstream.
                for from in &from_subtasks {
                    for to in &to_subtasks {
                        edges.push(PhysicalEdge {
                            from: from.subtask_id.clone(),
                            to: to.subtask_id.clone(),
                            partition_strategy: logical_edge.partition_strategy.clone(),
                        });
                    }
                }
            }
            PartitionStrategy::Broadcast => {
                // Every upstream sends to every downstream.
                for from in &from_subtasks {
                    for to in &to_subtasks {
                        edges.push(PhysicalEdge {
                            from: from.subtask_id.clone(),
                            to: to.subtask_id.clone(),
                            partition_strategy: PartitionStrategy::Broadcast,
                        });
                    }
                }
            }
        }
    }

    edges
}

#[cfg(test)]
mod tests {
    use crate::dag::{LogicalGraph, NodeKind, PartitionStrategy};

    use super::*;

    fn two_tm_slots() -> Vec<SlotOffer> {
        vec![
            SlotOffer {
                task_manager_id: "tm-1".into(),
                total_slots: 4,
                used_slots: 0,
            },
            SlotOffer {
                task_manager_id: "tm-2".into(),
                total_slots: 4,
                used_slots: 0,
            },
        ]
    }

    fn linear_graph() -> LogicalGraph {
        let mut g = LogicalGraph::new();
        let src = g.add_node("source", NodeKind::Source, 1);
        let map = g.add_node("map", NodeKind::Operator, 4);
        let sink = g.add_node("sink", NodeKind::Sink, 2);
        g.add_edge(src, map, PartitionStrategy::Hash);
        g.add_edge(map, sink, PartitionStrategy::Forward);
        g
    }

    #[test]
    fn test_schedule_spread() {
        let graph = linear_graph();
        let slots = two_tm_slots();
        let pg = schedule(&graph, &slots, SchedulingStrategy::Spread).unwrap();

        // 1 + 4 + 2 = 7 subtasks
        assert_eq!(pg.assignments.len(), 7);

        // Spread: subtasks should be distributed across both TMs.
        let tm1_count = pg
            .assignments
            .iter()
            .filter(|a| a.task_manager_id == "tm-1")
            .count();
        let tm2_count = pg
            .assignments
            .iter()
            .filter(|a| a.task_manager_id == "tm-2")
            .count();
        // With spread on 2 TMs of 4 slots each and 7 subtasks,
        // expect roughly even distribution.
        assert!((3..=4).contains(&tm1_count));
        assert!((3..=4).contains(&tm2_count));
    }

    #[test]
    fn test_schedule_pack() {
        let graph = linear_graph();
        let slots = two_tm_slots();
        let pg = schedule(&graph, &slots, SchedulingStrategy::Pack).unwrap();

        assert_eq!(pg.assignments.len(), 7);

        // Pack: one TM should be filled to capacity (4), the other gets 3.
        let mut counts: Vec<usize> = vec![
            pg.assignments
                .iter()
                .filter(|a| a.task_manager_id == "tm-1")
                .count(),
            pg.assignments
                .iter()
                .filter(|a| a.task_manager_id == "tm-2")
                .count(),
        ];
        counts.sort();
        assert_eq!(counts, vec![3, 4]);
    }

    #[test]
    fn test_insufficient_slots() {
        let graph = linear_graph();
        let slots = vec![SlotOffer {
            task_manager_id: "tm-1".into(),
            total_slots: 3,
            used_slots: 0,
        }];
        let result = schedule(&graph, &slots, SchedulingStrategy::Spread);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not enough slots"));
    }

    #[test]
    fn test_forward_edges() {
        let mut g = LogicalGraph::new();
        let src = g.add_node("source", NodeKind::Source, 2);
        let sink = g.add_node("sink", NodeKind::Sink, 2);
        g.add_edge(src, sink, PartitionStrategy::Forward);

        let slots = two_tm_slots();
        let pg = schedule(&g, &slots, SchedulingStrategy::Spread).unwrap();

        // Forward with same parallelism: 2 edges (source-0->sink-0, source-1->sink-1).
        assert_eq!(pg.edges.len(), 2);
        for edge in &pg.edges {
            assert_eq!(edge.from.subtask_index, edge.to.subtask_index);
        }
    }

    #[test]
    fn test_hash_edges_all_to_all() {
        let mut g = LogicalGraph::new();
        let src = g.add_node("source", NodeKind::Source, 2);
        let map = g.add_node("map", NodeKind::Operator, 3);
        g.add_edge(src, map, PartitionStrategy::Hash);

        let slots = vec![SlotOffer {
            task_manager_id: "tm-1".into(),
            total_slots: 5,
            used_slots: 0,
        }];
        let pg = schedule(&g, &slots, SchedulingStrategy::Pack).unwrap();

        // Hash: 2 sources * 3 maps = 6 edges.
        assert_eq!(pg.edges.len(), 6);
    }

    #[test]
    fn test_subtask_id_display() {
        let id = SubtaskId::new(0, "map", 2);
        assert_eq!(id.to_string(), "map-2");
        assert_eq!(id.canonical_name(), "map-2");
    }

    #[test]
    fn test_physical_graph_serde_roundtrip() {
        let graph = linear_graph();
        let slots = two_tm_slots();
        let pg = schedule(&graph, &slots, SchedulingStrategy::Spread).unwrap();

        let bytes = bitcode::serialize(&pg).unwrap();
        let pg2: PhysicalGraph = bitcode::deserialize(&bytes).unwrap();
        assert_eq!(pg2.assignments.len(), pg.assignments.len());
        assert_eq!(pg2.edges.len(), pg.edges.len());
    }
}
