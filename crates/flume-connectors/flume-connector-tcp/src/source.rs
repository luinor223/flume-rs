//! TCP source — reads line-delimited text from a TCP connection.

use std::future::Future;
use std::pin::Pin;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::net::tcp::OwnedReadHalf;
use tokio::net::{TcpStream, ToSocketAddrs};

use flume_core::{EventTimestamp, FlumeResult, Record, Source, StreamElement};

/// Reads line-delimited text from a TCP connection.
///
/// Operates in client mode — connects to a remote address.
/// No meaningful checkpoint position (live stream).
pub struct TcpSource {
    reader: BufReader<OwnedReadHalf>,
    line_buf: String,
}

impl TcpSource {
    /// Connect to a TCP server and prepare to read lines.
    pub async fn connect(addr: impl ToSocketAddrs) -> FlumeResult<Self> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| flume_core::FlumeError::Source(Box::new(e)))?;
        let (read_half, _write_half) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(read_half),
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

impl Source<String> for TcpSource {
    fn next(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = FlumeResult<Option<StreamElement<String>>>> + Send + '_>> {
        Box::pin(async {
            self.line_buf.clear();
            let n = self
                .reader
                .read_line(&mut self.line_buf)
                .await
                .map_err(|e| flume_core::FlumeError::Source(Box::new(e)))?;
            if n == 0 {
                return Ok(None);
            }
            let line = self.line_buf.trim_end_matches('\n').trim_end_matches('\r');
            Ok(Some(StreamElement::Record(Record::new(
                line.to_owned(),
                now_millis(),
            ))))
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
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn test_read_lines_from_tcp() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Spawn a server that sends two lines then closes
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            stream.write_all(b"hello\nworld\n").await.unwrap();
            stream.shutdown().await.unwrap();
        });

        let mut source = TcpSource::connect(addr).await.unwrap();

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

        // Connection closed → None
        assert!(source.next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_snapshot_returns_empty() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
        });

        let source = TcpSource::connect(addr).await.unwrap();
        let state = source.snapshot().await.unwrap();
        assert!(state.is_empty());
    }
}
