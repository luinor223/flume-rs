//! File source — reads line-delimited text from a file.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, BufReader};

use flume_core::{EventTimestamp, FlumeResult, Record, Source, StreamElement};

/// Reads a file line by line, emitting each line as a `Record<String>`.
///
/// Supports checkpoint/restore by tracking the byte offset.
pub struct FileSource {
    path: PathBuf,
    reader: BufReader<File>,
    bytes_read: u64,
    line_buf: String,
}

impl FileSource {
    /// Open a file for reading.
    pub async fn new(path: impl AsRef<Path>) -> FlumeResult<Self> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path).await?;
        Ok(Self {
            path,
            reader: BufReader::new(file),
            bytes_read: 0,
            line_buf: String::new(),
        })
    }
}

fn now_millis() -> EventTimestamp {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    EventTimestamp::new(millis)
}

impl Source<String> for FileSource {
    fn next(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = FlumeResult<Option<StreamElement<String>>>> + Send + '_>> {
        Box::pin(async {
            self.line_buf.clear();
            let n = self.reader.read_line(&mut self.line_buf).await?;
            if n == 0 {
                return Ok(None);
            }
            self.bytes_read += n as u64;
            let line = self.line_buf.trim_end_matches('\n').trim_end_matches('\r');
            Ok(Some(StreamElement::Record(Record::new(
                line.to_owned(),
                now_millis(),
            ))))
        })
    }

    fn snapshot(&self) -> Pin<Box<dyn Future<Output = FlumeResult<Vec<u8>>> + Send + '_>> {
        Box::pin(async { Ok(self.bytes_read.to_le_bytes().to_vec()) })
    }

    fn restore(
        &mut self,
        state: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = FlumeResult<()>> + Send + '_>> {
        Box::pin(async move {
            let bytes: [u8; 8] = state
                .try_into()
                .map_err(|_| flume_core::FlumeError::Checkpoint("invalid offset".into()))?;
            let offset = u64::from_le_bytes(bytes);
            let mut file = File::open(&self.path).await?;
            file.seek(std::io::SeekFrom::Start(offset)).await?;
            self.reader = BufReader::new(file);
            self.bytes_read = offset;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_read_lines() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "hello").unwrap();
        writeln!(tmp, "world").unwrap();
        tmp.flush().unwrap();

        let mut source = FileSource::new(tmp.path()).await.unwrap();

        let elem = source.next().await.unwrap().unwrap();
        if let StreamElement::Record(r) = elem {
            assert_eq!(r.value, "hello");
        } else {
            panic!("expected Record");
        }

        let elem = source.next().await.unwrap().unwrap();
        if let StreamElement::Record(r) = elem {
            assert_eq!(r.value, "world");
        } else {
            panic!("expected Record");
        }

        assert!(source.next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_snapshot_restore() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "line1").unwrap();
        writeln!(tmp, "line2").unwrap();
        writeln!(tmp, "line3").unwrap();
        tmp.flush().unwrap();

        let mut source = FileSource::new(tmp.path()).await.unwrap();

        // Read first line
        source.next().await.unwrap();

        // Snapshot after first line
        let state = source.snapshot().await.unwrap();

        // Read second line
        let elem = source.next().await.unwrap().unwrap();
        if let StreamElement::Record(r) = &elem {
            assert_eq!(r.value, "line2");
        }

        // Restore to after first line
        source.restore(state).await.unwrap();

        // Should re-read line2
        let elem = source.next().await.unwrap().unwrap();
        if let StreamElement::Record(r) = elem {
            assert_eq!(r.value, "line2");
        } else {
            panic!("expected Record");
        }
    }

    #[tokio::test]
    async fn test_eof_returns_none() {
        let tmp = NamedTempFile::new().unwrap();
        let mut source = FileSource::new(tmp.path()).await.unwrap();
        assert!(source.next().await.unwrap().is_none());
    }
}
