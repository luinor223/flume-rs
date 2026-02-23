//! File sink — writes line-delimited text to a file.

use std::fmt::Display;
use std::future::Future;
use std::marker::PhantomData;
use std::path::Path;
use std::pin::Pin;

use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};

use flume_core::{FlumeResult, Record, Sink};

/// Writes records as newline-delimited text to a file.
///
/// Each record's value is converted via `Display` and written as one line.
pub struct FileSink<T: Display + Send + 'static> {
    writer: BufWriter<File>,
    _marker: PhantomData<T>,
}

impl<T: Display + Send + 'static> FileSink<T> {
    /// Create a new file sink, creating or truncating the file at `path`.
    pub async fn new(path: impl AsRef<Path>) -> FlumeResult<Self> {
        let file = File::create(path).await?;
        Ok(Self {
            writer: BufWriter::new(file),
            _marker: PhantomData,
        })
    }
}

impl<T: Display + Send + 'static> Sink<T> for FileSink<T> {
    fn write(
        &mut self,
        record: Record<T>,
    ) -> Pin<Box<dyn Future<Output = FlumeResult<()>> + Send + '_>> {
        Box::pin(async move {
            let line = format!("{}\n", record.value);
            self.writer.write_all(line.as_bytes()).await?;
            Ok(())
        })
    }

    fn flush(&mut self) -> Pin<Box<dyn Future<Output = FlumeResult<()>> + Send + '_>> {
        Box::pin(async {
            self.writer.flush().await?;
            Ok(())
        })
    }

    fn snapshot(&self) -> Pin<Box<dyn Future<Output = FlumeResult<Vec<u8>>> + Send + '_>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn restore(
        &mut self,
        _state: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = FlumeResult<()>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flume_core::EventTimestamp;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_write_lines() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        {
            let mut sink: FileSink<String> = FileSink::new(&path).await.unwrap();
            sink.write(Record::new("hello".to_string(), EventTimestamp::new(1)))
                .await
                .unwrap();
            sink.write(Record::new("world".to_string(), EventTimestamp::new(2)))
                .await
                .unwrap();
            sink.flush().await.unwrap();
        }

        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "hello\nworld\n");
    }

    #[tokio::test]
    async fn test_write_integers() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        {
            let mut sink: FileSink<i32> = FileSink::new(&path).await.unwrap();
            sink.write(Record::new(42, EventTimestamp::new(1)))
                .await
                .unwrap();
            sink.write(Record::new(99, EventTimestamp::new(2)))
                .await
                .unwrap();
            sink.flush().await.unwrap();
        }

        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "42\n99\n");
    }

    #[tokio::test]
    async fn test_snapshot_returns_empty() {
        let tmp = NamedTempFile::new().unwrap();
        let sink: FileSink<String> = FileSink::new(tmp.path()).await.unwrap();
        let state = sink.snapshot().await.unwrap();
        assert!(state.is_empty());
    }
}
