//! TCP connector for the flume-rs stream processing engine.
//!
//! Provides line-delimited text TCP source and sink (client-mode).

mod sink;
mod source;

pub use sink::TcpSink;
pub use source::TcpSource;
