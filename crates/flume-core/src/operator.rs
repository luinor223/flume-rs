//! Built-in operator implementations.

mod filter;
mod flat_map;
mod map;

pub use filter::FilterOperator;
pub use flat_map::FlatMapOperator;
pub use map::MapOperator;
