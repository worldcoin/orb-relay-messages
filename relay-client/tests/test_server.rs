use orb_relay_messages::relay::{
    relay_connect_request,
    relay_connect_response::{self, Msg},
    relay_service_server::{RelayService, RelayServiceServer},
    Ack, ConnectRequest, ConnectResponse, Entity, RelayConnectRequest,
    RelayConnectResponse, RelayPayload,
};
use std::{collections::HashMap, net::SocketAddr, pin::Pin, sync::Arc};
use tokio::{
    net::TcpListener,
    sync::{Mutex, MutexGuard},
    task,
};
use tokio_stream::{wrappers::TcpListenerStream, Stream};
use tonic::{transport::Server, Request, Response, Status, Streaming};

type OutboundMessageStream =
    Pin<Box<dyn Stream<Item = Result<RelayConnectResponse, Status>> + Send>>;

#[derive(Default, Clone)]
pub struct ConnectedClients {
    clients: Arc<Mutex<HashMap<String, flume::Sender<Cmd>>>>,
}

impl ConnectedClients {
    async fn insert(&self, entity: &Entity, tx: flume::Sender<Cmd>) {
        let mut clients = self.clients.lock().await;
        let key = entity_key(entity);
        clients.insert(key, tx.clone());
    }

    #[allow(dead_code)] // wtf, this is actually used lol...
    pub fn send(&self, payload: RelayPayload) {
        let clients = self.clients.clone();
        task::spawn(async move {
            let clients = clients.lock().await;
            let entity_key = entity_key(payload.dst.as_ref().unwrap());
            if let Some(tx) = clients.get(&entity_key) {
                let _ = tx.send(Cmd::Send(payload));
            }
        });
    }

    #[allow(dead_code)]
    pub fn stop(&self, entity: &Entity) {
        let clients = self.clients.clone();
        let entity_key = entity_key(&entity);

        task::spawn(async move {
            let clients = clients.lock().await;
            if let Some(tx) = clients.get(&entity_key) {
                let _ = tx.send(Cmd::Terminate);
            }
        });
    }
}

struct RelayServiceHandler<S>
where
    S: Send + Sync + 'static,
{
    state: Arc<Mutex<S>>,
    clients: ConnectedClients,
    handler: Arc<
        dyn Fn(
                &mut S,
                relay_connect_request::Msg,
                &ConnectedClients,
            ) -> Option<RelayConnectResponse>
            + Send
            + Sync
            + 'static,
    >,
}

enum Cmd {
    Send(RelayPayload),
    Terminate,
}

fn entity_key(e: &Entity) -> String {
    format!("{}{}{}", e.id, e.namespace, e.entity_type)
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
        let (tx, rx) = flume::unbounded();
        let mut stream = request.into_inner();
        let state = self.state.clone();
        let handler = self.handler.clone();
        let clients = self.clients.clone();

        let output = async_stream::stream! {
            loop {
                let handled_replies = async {
                    let msg = stream.message().await.unwrap().unwrap().msg.unwrap();

                    if let relay_connect_request::Msg::ConnectRequest(ConnectRequest { client_id, ..   }) =  &msg {
                        let entity = client_id.as_ref().unwrap();
                        clients.insert(entity, tx.clone()).await;
                    }

                    let mut state = state.lock().await;
                    (handler)(&mut state, msg, &clients)
                };


                tokio::select! {
                    msg = handled_replies => {
                        if let Some(m) = msg {
                            yield Ok(m)
                        }
                    }

                    msg = rx.recv_async() => {
                        match msg.unwrap() {
                            Cmd::Send(payload) => {
                                let res = RelayConnectResponse {
                                    msg: Some(relay_connect_response::Msg::Payload(payload)),
                                };

                                yield Ok(res)
                            }

                            Cmd::Terminate => {
                                break;
                            }
                        }
                    }
                }
            }
        };

        Ok(Response::new(Box::pin(output) as Self::RelayConnectStream))
    }
}

/// A test server that spawns a real gRPC server instance.
/// This server will listen on a random local port and can be used
/// for integration testing of gRPC clients. The server runs in a
/// separate tokio task and will be shut down when this struct is dropped.
pub struct TestServer<S> {
    addr: SocketAddr,
    #[allow(dead_code)] // not dead code but clippy complains eh
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
    /// Creates a new `TestServer` with an initial state and handler function.
    ///
    /// The handler receives mutable access to the server state and each incoming request message.
    /// It can modify the state and returns an optional response message.
    ///
    /// The server's state can be accessed at any time by calling `self.state()`, which returns
    /// a MutexGuard containing the state.
    ///
    /// # Example
    /// ```ignore
    /// let initial_state = 0;
    /// let sv = TestServer::new(initial_state, |state, conn_req| {
    ///     *state += 1;
    ///     match conn_req {
    ///         Msg::ConnectRequest(ConnectRequest { client_id, .. }) => ConnectResponse {
    ///             client_id: client_id.unwrap().id.clone(),
    ///             success: true,
    ///             error: "nothing".to_string(),
    ///         }
    ///         .into_res(),
    ///
    ///         _ => None,
    ///     }
    /// })
    /// .await;
    ///
    /// let state = sv.state().await;
    /// ```
    pub async fn new<F>(state: S, handler: F) -> Self
    where
        F: Fn(
                &mut S,
                relay_connect_request::Msg,
                &ConnectedClients,
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
                    clients: ConnectedClients::default(),
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

    /// Gets the server's random TCP socket address
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    #[allow(dead_code)] // this is not dead code why do i need this
    /// Returns a mutex guard that can be used to access and modify the server's state
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
