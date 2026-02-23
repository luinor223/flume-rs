//! Logical graph (DAG) representation for pipeline execution.

use std::collections::VecDeque;

/// Unique identifier for a node in the logical graph.
pub type NodeId = usize;

/// The kind of processing a node performs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Source,
    Operator,
    Sink,
}

/// How records are routed between parallel subtasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartitionStrategy {
    /// 1:1 index mapping between upstream and downstream subtasks.
    Forward,
    /// Route by `hash(record.key) % num_targets`.
    Hash,
    /// Send a copy to every downstream subtask.
    Broadcast,
    /// Round-robin across downstream subtasks.
    Rebalance,
}

/// A node in the logical graph.
#[derive(Debug, Clone)]
pub struct LogicalNode {
    pub id: NodeId,
    pub name: String,
    pub kind: NodeKind,
    pub parallelism: usize,
}

/// A directed edge in the logical graph.
#[derive(Debug, Clone)]
pub struct LogicalEdge {
    pub from: NodeId,
    pub to: NodeId,
    pub partition_strategy: PartitionStrategy,
}

/// The directed acyclic graph representing a pipeline before execution.
#[derive(Debug, Default)]
pub struct LogicalGraph {
    pub nodes: Vec<LogicalNode>,
    pub edges: Vec<LogicalEdge>,
}

impl LogicalGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a node and return its ID.
    pub fn add_node(
        &mut self,
        name: impl Into<String>,
        kind: NodeKind,
        parallelism: usize,
    ) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(LogicalNode {
            id,
            name: name.into(),
            kind,
            parallelism,
        });
        id
    }

    /// Add a directed edge between two nodes.
    pub fn add_edge(&mut self, from: NodeId, to: NodeId, partition_strategy: PartitionStrategy) {
        self.edges.push(LogicalEdge {
            from,
            to,
            partition_strategy,
        });
    }

    /// Return node IDs in topological order (sources first, sinks last).
    ///
    /// Returns `None` if the graph contains a cycle.
    pub fn topological_order(&self) -> Option<Vec<NodeId>> {
        let n = self.nodes.len();
        let mut in_degree = vec![0usize; n];
        let mut adjacency: Vec<Vec<NodeId>> = vec![Vec::new(); n];

        for edge in &self.edges {
            adjacency[edge.from].push(edge.to);
            in_degree[edge.to] += 1;
        }

        let mut queue: VecDeque<NodeId> = VecDeque::new();
        for (id, &deg) in in_degree.iter().enumerate() {
            if deg == 0 {
                queue.push_back(id);
            }
        }

        let mut order = Vec::with_capacity(n);
        while let Some(node) = queue.pop_front() {
            order.push(node);
            for &neighbor in &adjacency[node] {
                in_degree[neighbor] -= 1;
                if in_degree[neighbor] == 0 {
                    queue.push_back(neighbor);
                }
            }
        }

        if order.len() == n { Some(order) } else { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linear_pipeline() {
        let mut graph = LogicalGraph::new();
        let src = graph.add_node("source", NodeKind::Source, 1);
        let map = graph.add_node("map", NodeKind::Operator, 1);
        let sink = graph.add_node("sink", NodeKind::Sink, 1);

        graph.add_edge(src, map, PartitionStrategy::Forward);
        graph.add_edge(map, sink, PartitionStrategy::Forward);

        let order = graph.topological_order().unwrap();
        assert_eq!(order, vec![0, 1, 2]);
    }

    #[test]
    fn test_diamond_topology() {
        let mut graph = LogicalGraph::new();
        let src = graph.add_node("source", NodeKind::Source, 1);
        let left = graph.add_node("left", NodeKind::Operator, 1);
        let right = graph.add_node("right", NodeKind::Operator, 1);
        let sink = graph.add_node("sink", NodeKind::Sink, 1);

        graph.add_edge(src, left, PartitionStrategy::Forward);
        graph.add_edge(src, right, PartitionStrategy::Forward);
        graph.add_edge(left, sink, PartitionStrategy::Forward);
        graph.add_edge(right, sink, PartitionStrategy::Forward);

        let order = graph.topological_order().unwrap();
        // Source must come first, sink must come last
        assert_eq!(order[0], src);
        assert_eq!(order[3], sink);
    }

    #[test]
    fn test_cycle_detection() {
        let mut graph = LogicalGraph::new();
        let a = graph.add_node("a", NodeKind::Operator, 1);
        let b = graph.add_node("b", NodeKind::Operator, 1);

        graph.add_edge(a, b, PartitionStrategy::Forward);
        graph.add_edge(b, a, PartitionStrategy::Forward);

        assert!(graph.topological_order().is_none());
    }
}
