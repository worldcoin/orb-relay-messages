use orb_relay_messages::relay::{
    relay_connect_request,
    relay_connect_response::{self, Msg},
    relay_service_server::{RelayService, RelayServiceServer},
    Ack, ConnectResponse, RelayConnectRequest, RelayConnectResponse, RelayPayload,
};
use std::{net::SocketAddr, pin::Pin, sync::Arc};
use tokio::{
    net::TcpListener,
    sync::{Mutex, MutexGuard},
};
use tokio_stream::{wrappers::TcpListenerStream, Stream};
use tonic::{transport::Server, Request, Response, Status, Streaming};

type OutboundMessageStream =
    Pin<Box<dyn Stream<Item = Result<RelayConnectResponse, Status>> + Send>>;

struct RelayServiceHandler<S>
where
    S: Send + Sync + 'static,
{
    state: Arc<Mutex<S>>,
    handler: Arc<
        dyn Fn(
                &mut S,
                relay_connect_request::Msg,
            ) -> Option<RelayConnectResponse>
            + Send
            + Sync
            + 'static,
    >,
}

#[tonic::async_trait]
impl<S> RelayService for RelayServiceHandler<S>
where
    S: Send + Sync + 'static,
{
    type RelayConnectStream = OutboundMessageStream;

    async fn relay_connect(
        &self,
        request: Request<Streaming<RelayConnectRequest>>,
    ) -> Result<Response<Self::RelayConnectStream>, Status> {
        let mut stream = request.into_inner();
        let state = self.state.clone();
        let handler = self.handler.clone();

        let output = async_stream::stream! {
            loop {
                let msg = stream.message().await.unwrap().unwrap().msg.unwrap();
                let mut state = state.lock().await;
                if let Some(res) = (handler)(&mut state, msg) {
                    yield Ok(res)
                }
            }
        };

        Ok(Response::new(Box::pin(output) as Self::RelayConnectStream))
    }
}

pub struct TestServer<S> {
    addr: SocketAddr,
    state: Arc<Mutex<S>>,
    shutdown: flume::Sender<()>,
}

impl<S> Drop for TestServer<S> {
    fn drop(&mut self) {
        self.shutdown.send(()).unwrap();
    }
}

impl<S> TestServer<S>
where
    S: Sync + Send + 'static,
{
    pub async fn new<F>(state: S, handler: F) -> Self
    where
        F: Fn(
                &mut S,
                relay_connect_request::Msg,
            ) -> Option<RelayConnectResponse>
            + Send
            + Sync
            + 'static,
    {
        let handler = Arc::new(handler);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().map_err(Box::new).unwrap();
        let (tx, rx) = flume::bounded(1);

        let state = Arc::new(Mutex::new(state));
        tokio::spawn(
            Server::builder()
                .add_service(RelayServiceServer::new(RelayServiceHandler {
                    state: state.clone(),
                    handler,
                }))
                .serve_with_incoming_shutdown(
                    TcpListenerStream::new(listener),
                    async move {
                        rx.recv_async().await.ok();
                    },
                ),
        );

        TestServer {
            addr,
            shutdown: tx,
            state,
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub async fn state(&self) -> MutexGuard<'_, S> {
        self.state.lock().await
    }
}

pub trait IntoRes: Sized {
    fn into_res(self) -> Option<RelayConnectResponse>;
}

impl IntoRes for ConnectResponse {
    fn into_res(self) -> Option<RelayConnectResponse> {
        Some(RelayConnectResponse {
            msg: Some(Msg::ConnectResponse(self)),
        })
    }
}

impl IntoRes for Ack {
    fn into_res(self) -> Option<RelayConnectResponse> {
        Some(RelayConnectResponse {
            msg: Some(Msg::Ack(self)),
        })
    }
}

impl IntoRes for RelayPayload {
    fn into_res(self) -> Option<RelayConnectResponse> {
        Some(RelayConnectResponse {
            msg: Some(Msg::Payload(self)),
        })
    }
}

impl IntoRes for () {
    fn into_res(self) -> Option<RelayConnectResponse> {
        None
    }
}

impl IntoRes for Option<RelayConnectResponse> {
    fn into_res(self) -> Option<RelayConnectResponse> {
        self
    }
}
