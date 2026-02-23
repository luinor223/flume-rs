//! File connector for the flume-rs stream processing engine.
//!
//! Provides line-delimited text file source and sink.

mod sink;
mod source;

pub use sink::FileSink;
pub use source::FileSource;
