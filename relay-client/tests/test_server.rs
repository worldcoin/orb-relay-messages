use orb_relay_messages::relay::{
    relay_connect_response::Msg,
    relay_service_server::{RelayService, RelayServiceServer},
    Ack, ConnectResponse, RelayConnectRequest, RelayConnectResponse, RelayPayload,
};
use std::{net::SocketAddr, pin::Pin};
use tokio::net::TcpListener;
use tokio_stream::{wrappers::TcpListenerStream, Stream, StreamExt};
use tonic::{transport::Server, Request, Response, Status, Streaming};

type OutboundMessageStream =
    Pin<Box<dyn Stream<Item = Result<RelayConnectResponse, Status>> + Send>>;

struct RelayServiceHandler<T: IntoRelayConnectResponse + Sized>(
    Box<
        dyn Fn(Streaming<RelayConnectRequest>) -> Pin<Box<dyn Stream<Item = T> + Send>>
            + Send
            + Sync
            + 'static,
    >,
);

#[tonic::async_trait]
impl<T: IntoRelayConnectResponse + Send + Sync + 'static> RelayService
    for RelayServiceHandler<T>
{
    type RelayConnectStream = OutboundMessageStream;

    async fn relay_connect(
        &self,
        request: Request<Streaming<RelayConnectRequest>>,
    ) -> Result<Response<Self::RelayConnectStream>, Status> {
        let stream = request.into_inner();
        let handler_stream = (self.0)(stream);

        let output =
            Box::pin(handler_stream.map(|item| Ok(item.into_relay_connect_response())));

        Ok(Response::new(output))
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
    pub async fn new<F, T, S>(handler: F) -> TestServer
    where
        F: Fn(Streaming<RelayConnectRequest>) -> S,
        F: Send + Sync + 'static,
        S: Stream<Item = T> + Send + 'static,
        T: IntoRelayConnectResponse + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().map_err(Box::new).unwrap();
        let (tx, rx) = flume::bounded(1);

        let wrapped_handler = move |stream| Box::pin(handler(stream)) as Pin<Box<dyn Stream<Item = T> + Send>>;

        tokio::spawn(
            Server::builder()
                .add_service(RelayServiceServer::new(RelayServiceHandler(Box::new(
                    wrapped_handler,
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
