use orb_relay_messages::relay::{
    relay_connect_response::Msg,
    relay_service_server::{RelayService, RelayServiceServer},
    Ack, ConnectResponse, RelayConnectRequest, RelayConnectResponse, RelayPayload,
};
use std::{future, net::SocketAddr, pin::Pin, str::FromStr};
use tokio::net::TcpListener;
use tokio_stream::{wrappers::TcpListenerStream, Stream};
use tonic::{transport::Server, Request, Response, Status, Streaming};

type OutboundMessageStream =
    Pin<Box<dyn Stream<Item = Result<RelayConnectResponse, Status>> + Send>>;

struct RelayServiceHandler<T: IntoRelayConnectResponse>(
    Box<
        dyn Fn() -> Box<
                dyn FnMut(
                        RelayConnectRequest,
                    )
                        -> Pin<Box<dyn future::Future<Output = T> + Send>>
                    + Send,
            > + Send
            + Sync
            + 'static,
    >,
);

#[tonic::async_trait]
impl<T: IntoRelayConnectResponse + Send + 'static> RelayService
    for RelayServiceHandler<T>
{
    type RelayConnectStream = OutboundMessageStream;

    async fn relay_connect(
        &self,
        request: Request<Streaming<RelayConnectRequest>>,
    ) -> Result<Response<Self::RelayConnectStream>, Status> {
        let mut handler = (self.0)();
        let mut stream = request.into_inner();

        let output = async_stream::stream! {
            let msg = stream.message().await.unwrap().unwrap();
            yield Ok(handler(msg).await.into_relay_connect_response());
        };

        Ok(Response::new(Box::pin(output) as Self::RelayConnectStream))
    }
}

pub struct TestServer {
    pub addr: SocketAddr,
    shutdown: flume::Sender<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown.send(()).unwrap();
    }
}

impl TestServer {
    /// Creates a new relay service that handles relay connect requests for testing.
    /// Uses flume channels for shutdown signaling.
    ///
    /// Given closure is called on every new connection, and the inner `FnMut()` called on every received [`RelayConnectRequest`] for that connection.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let server = TestServer::new(|| {
    ///     let mut state = 0;
    ///     |relay_conn_request| async move {
    ///         println!("received msg number: {state}");
    ///         state += 1;
    ///         ConnectResponse {
    ///             client_id: "bla".to_string(),
    ///             success: true,
    ///             error: String::default(),
    ///         }
    ///     }
    /// }).await;
    /// ```
    pub async fn new<F, H, Fut, T>(handler: F) -> TestServer
    where
        F: Fn() -> H + Send + Sync + 'static,
        H: FnMut(RelayConnectRequest) -> Fut + Send + 'static,
        Fut: future::Future<Output = T> + Send + 'static,
        T: IntoRelayConnectResponse + Send + 'static,
    {
        let handler = move || {
            let mut inner_handler = (handler)();
            let boxed: Box<
                dyn FnMut(
                        RelayConnectRequest,
                    )
                        -> Pin<Box<dyn future::Future<Output = T> + Send>>
                    + Send,
            > = Box::new(move |req| {
                let fut = inner_handler(req);
                let pinned: Pin<Box<dyn future::Future<Output = T> + Send>> =
                    Box::pin(fut);
                pinned
            });
            boxed
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().map_err(Box::new).unwrap();
        let (tx, rx) = flume::bounded(1);

        tokio::spawn(
            Server::builder()
                .add_service(RelayServiceServer::new(RelayServiceHandler(Box::new(
                    handler,
                ))))
                .serve_with_incoming_shutdown(
                    TcpListenerStream::new(listener),
                    async move {
                        rx.recv_async().await.ok();
                    },
                ),
        );

        TestServer { addr, shutdown: tx }
    }
}

pub trait IntoRelayConnectResponse {
    fn into_relay_connect_response(self) -> RelayConnectResponse;
}

impl IntoRelayConnectResponse for Ack {
    fn into_relay_connect_response(self) -> RelayConnectResponse {
        RelayConnectResponse {
            msg: Some(Msg::Ack(self)),
        }
    }
}

impl IntoRelayConnectResponse for ConnectResponse {
    fn into_relay_connect_response(self) -> RelayConnectResponse {
        RelayConnectResponse {
            msg: Some(Msg::ConnectResponse(self)),
        }
    }
}

impl IntoRelayConnectResponse for RelayPayload {
    fn into_relay_connect_response(self) -> RelayConnectResponse {
        RelayConnectResponse {
            msg: Some(Msg::Payload(self)),
        }
    }
}
