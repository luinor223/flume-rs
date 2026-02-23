//! TCP sink — writes line-delimited text to a TCP connection.

use std::fmt::Display;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;

use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpStream, ToSocketAddrs};

use flume_core::{FlumeResult, Record, Sink};

/// Writes records as newline-delimited text to a TCP connection.
///
/// Operates in client mode — connects to a remote address.
pub struct TcpSink<T: Display + Send + 'static> {
    writer: BufWriter<OwnedWriteHalf>,
    _marker: PhantomData<T>,
}

impl<T: Display + Send + 'static> TcpSink<T> {
    /// Connect to a TCP server and prepare to write lines.
    pub async fn connect(addr: impl ToSocketAddrs) -> FlumeResult<Self> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| flume_core::FlumeError::Sink(Box::new(e)))?;
        let (_read_half, write_half) = stream.into_split();
        Ok(Self {
            writer: BufWriter::new(write_half),
            _marker: PhantomData,
        })
    }
}

impl<T: Display + Send + 'static> Sink<T> for TcpSink<T> {
    fn write(
        &mut self,
        record: Record<T>,
    ) -> Pin<Box<dyn Future<Output = FlumeResult<()>> + Send + '_>> {
        Box::pin(async move {
            let line = format!("{}\n", record.value);
            self.writer
                .write_all(line.as_bytes())
                .await
                .map_err(|e| flume_core::FlumeError::Sink(Box::new(e)))?;
            Ok(())
        })
    }

    fn flush(&mut self) -> Pin<Box<dyn Future<Output = FlumeResult<()>> + Send + '_>> {
        Box::pin(async {
            self.writer
                .flush()
                .await
                .map_err(|e| flume_core::FlumeError::Sink(Box::new(e)))?;
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
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn test_write_lines_to_tcp() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = String::new();
            stream.read_to_string(&mut buf).await.unwrap();
            buf
        });

        let mut sink: TcpSink<String> = TcpSink::connect(addr).await.unwrap();
        sink.write(Record::new("hello".to_string(), EventTimestamp::new(1)))
            .await
            .unwrap();
        sink.write(Record::new("world".to_string(), EventTimestamp::new(2)))
            .await
            .unwrap();
        sink.flush().await.unwrap();
        // Drop sink to close connection so server reads EOF
        drop(sink);

        let received = server.await.unwrap();
        assert_eq!(received, "hello\nworld\n");
    }
}
