//! Built-in operator implementations.

mod filter;
mod flat_map;
mod map;
mod process;
mod window;

pub use self::process::{KeyedProcessOperator, ProcessOperator};
pub use self::window::WindowOperator;
pub use filter::FilterOperator;
pub use flat_map::FlatMapOperator;
pub use map::MapOperator;
