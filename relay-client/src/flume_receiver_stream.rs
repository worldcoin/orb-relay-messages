// tokio::sync::mpsc::Receiver sucks
// tonic is tied to tokio's Receiver unfortunately 🤦

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

/// Creates a `tokio_stream::wrappers::ReceiverStream` from a `flume::Receiver<_>`
/// ## example
/// ```ignore
/// let (tx, rx) = fume::unbounded();
/// let recv_stream = flume_receiver_stream::new(rx.clone(), 4);
/// ```
pub fn new<T: Send + 'static>(
    flume_rx: flume::Receiver<T>,
    tokio_mpsc_receiver_buffer: usize,
) -> ReceiverStream<T> {
    let (tx, rx) = mpsc::channel(tokio_mpsc_receiver_buffer);

    tokio::spawn(async move {
        while let Ok(msg) = flume_rx.recv_async().await {
            if tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    ReceiverStream::new(rx)
}
