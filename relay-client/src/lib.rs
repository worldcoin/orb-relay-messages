use bon::Builder;
use color_eyre::eyre::{self, Context};
use derive_more::From;
use orb_relay_messages::prost_types::Any;
use orb_relay_messages::relay::{entity::EntityType, Ack, Entity, RelayPayload};
use secrecy::SecretString;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch::Receiver;
use tokio::task::JoinHandle;
use tonic::transport;

pub use amount::Amount;

mod actor;
mod amount;
mod flume_receiver_stream;
mod tls;

pub type ClientId = String;
pub type Seq = u64;

/// A message to be sent to a another client through `orb-relay`.
/// QoS (Quality of Service) is optional and defaults to `QoS::AtMostOnce`.
///
/// # Example
/// ```ignore
/// // Create a message to send to a device
/// let msg = SendMessage::to(EntityType::Device)
///     .id("device_123")
///     .namespace("default")
///     .qos(QoS::AtLeastOnce) // Optional, defaults to AtMostOnce
///     .payload(b"hello");
///
/// client.send(msg).await.unwrap();
/// ```
#[derive(Debug, Builder, Clone)]
#[builder(on(String, into))]
#[builder(on(Vec<u8>, into))]
#[builder(on(Vec<u8>, into))]
#[builder(start_fn = to)]
#[builder(finish_fn = payload)]
pub struct SendMessage {
    #[builder(start_fn)]
    target_type: EntityType,
    #[builder(finish_fn)]
    data: Vec<u8>,
    /// Protobuf Any type URL; empty for untyped byte messages.
    #[builder(default)]
    type_url: String,
    #[builder(setters(name = id))]
    target_id: String,
    #[builder(setters(name = namespace))]
    target_namespace: String,
    #[builder(default = QoS::AtMostOnce)]
    qos: QoS,
}

/// Authentication options for the client.
///
/// - `Token`: Authentication using a secret token string
/// - `Zkp`: Zero Knowledge Proof authentication (TODO: not yet implemented)
///
/// # Example
/// ```ignore
/// // Authentication with a token:
/// let auth = Auth::Token("your_secure_token".into());
/// ```
#[derive(From, Debug, Clone)]
pub enum Auth {
    Token(SecretString),
    TokenReceiver(Receiver<String>),
    Zkp,
}

/// QoS delivery guarantees:
/// - AtMostOnce: Single send attempt without guarantees
/// - AtLeastOnce: Guaranteed delivery with retries (configurable via max_message_attempts) and acks
#[derive(Debug, Copy, Clone)]
pub enum QoS {
    AtMostOnce,
    AtLeastOnce,
}

pub(crate) struct RecvdRelayPayload {
    pub from: Entity,
    pub payload: Vec<u8>,
    pub type_url: String,
    pub to: Option<Entity>,
    pub seq: Seq,
}

#[derive(Debug)]
pub struct RecvMessage {
    pub from: Entity,
    pub payload: Vec<u8>,
    pub type_url: String,
    pub to: Option<Entity>,
    client: Client,
    seq: Seq,
}

impl RecvMessage {
    /// Recover the complete relay envelope for typed decoding and route validation.
    pub fn into_relay_payload(self) -> RelayPayload {
        RelayPayload {
            src: Some(self.from),
            dst: self.to,
            seq: self.seq,
            payload: Some(Any {
                type_url: self.type_url,
                value: self.payload,
            }),
        }
    }

    /// Reply to the sender of this message with a payload and specified QoS level.
    ///
    /// # Example
    /// ```ignore
    /// async fn handle_message(client: &Client) {
    ///     while let Ok(msg) = client.recv().await {
    ///         // Reply with "hello" using at-most-once delivery
    ///         msg.reply(b"hello", QoS::AtMostOnce).await.unwrap();
    ///
    ///         // Reply with bytes using at-least-once delivery
    ///         msg.reply(vec![1, 2, 3], QoS::AtLeastOnce).await.unwrap();
    ///     }
    /// }
    /// ```
    pub async fn reply(
        &self,
        payload: impl Into<Vec<u8>>,
        qos: QoS,
    ) -> Result<(), Err> {
        self.reply_any(
            Any {
                type_url: String::new(),
                value: payload.into(),
            },
            qos,
        )
        .await
    }

    /// Reply with a typed Protobuf payload, preserving its type URL on retries.
    pub async fn reply_any(&self, payload: Any, qos: QoS) -> Result<(), Err> {
        let relay_payload = RelayPayload {
            src: Some(Entity {
                id: self.client.opts.client_id.clone(),
                entity_type: self.client.opts.entity_type as i32,
                namespace: self.client.opts.namespace.clone(),
            }),
            dst: Some(Entity {
                id: self.from.id.clone(),
                entity_type: self.from.entity_type,
                namespace: self.from.namespace.clone(),
            }),
            seq: self.seq,
            payload: Some(payload.clone()),
        };

        if let QoS::AtLeastOnce = qos {
            let (ack_tx, ack_rx) = flume::unbounded();

            self.client
                .actor_tx
                .send(actor::Msg::WaitForAck {
                    original_msg: SendMessage {
                        target_type: EntityType::try_from(self.from.entity_type)
                            .wrap_err("Failed to convert EntityType")?,
                        data: payload.value,
                        type_url: payload.type_url,
                        target_id: self.from.id.clone(),
                        target_namespace: self.from.namespace.clone(),
                        qos,
                    },
                    ack_tx,
                    seq: self.seq,
                })
                .wrap_err("Error reaching actor loop")?;

            ack_rx
                .recv_async()
                .await
                .wrap_err("Error when waiting for ack")??;
        } else {
            self.client
                .actor_tx
                .send(actor::Msg::Send(relay_payload.into()))
                .wrap_err("Error reaching actor loop")?;
        };

        Ok(())
    }
}

#[derive(From, Debug)]
pub enum Err {
    AckTimeout,
    ReplyTimeout,
    ConnectionTimeout,
    AuthenticationFailed,
    UnexpectedResponse,
    StopRequest,
    ReconnectRequest,
    StreamEnded,
    Channel(flume::RecvError),
    Transport(transport::Error),
    Tonic(tonic::Status),
    Other(eyre::Error),
}

#[derive(Debug, Builder, Clone)]
#[builder(on(String, into))]
/// A client for sending and receiving messages through a `orb-relay` using message queues.
///
/// # Example
/// ```ignore
/// use orb_relay_client::{Client, ClientOpts};
///
/// let opts = ClientOpts::entity()
///     .entity_type(EntityType::Device)
///     .id("device_1")
///     .namespace("default")
///     .endpoint("https://relay.example.com")
///     .auth(Auth::Token("token123".into()))
///     .build();
///
/// let (client, handle) = Client::connect(opts);
/// ```
pub struct Client {
    opts: Arc<ClientOpts>,
    seq: Arc<AtomicU64>,
    #[builder(setters(vis = "pub(crate)"))]
    client_rx: flume::Receiver<RecvdRelayPayload>,
    #[builder(setters(vis = "pub(crate)"))]
    actor_tx: flume::Sender<actor::Msg>,
}

/// Options for configuring an `orb-relay` Client.
///
/// # Example
/// ```ignore
/// let opts = ClientOpts::entity()
///     // Required fields:
///     .entity_type(EntityType::Orb)
///     .id("device_1")
///     .namespace("default")
///     .endpoint("https://relay.example.com")
///     .auth(Auth::Token("token123".to_string()))
///     // Optional fields (defaults provided):
///     .connection_timeout(Duration::from_secs(30))
///     .max_connection_attempts(Amount::Infinite)
///     .ack_timeout(Duration::from_secs(20))
///     .reply_timeout(Duration::from_secs(20))
///     .heartbeat(Duration::from_secs(30))
///     .max_message_attempts(Amount::Infinite)
///     .build();
/// ```
#[allow(clippy::duplicated_attributes)] // clippy fail
#[derive(Debug, Builder, Clone)]
#[builder(on(String, into))]
#[builder(on(Auth, into))]
#[builder(start_fn = entity)]
pub struct ClientOpts {
    #[builder(start_fn)]
    entity_type: EntityType,
    #[builder(setters(name = id))]
    client_id: String,
    namespace: String,
    endpoint: String,
    auth: Auth,
    #[builder(default = Duration::from_secs(20))]
    connection_timeout: Duration,
    #[builder(default = Duration::from_secs(20))]
    connection_backoff: Duration,
    #[builder(default = Amount::Infinite)]
    max_connection_attempts: Amount,
    #[builder(default = Duration::from_secs(20))]
    ack_timeout: Duration,
    #[builder(default = Duration::from_secs(20))]
    reply_timeout: Duration,
    #[builder(default = Duration::from_secs(30))]
    heartbeat: Duration,
    #[builder(default = Amount::Infinite)]
    // Maximum retransmissions after the initial send; zero disables retries.
    max_message_attempts: Amount,
    #[builder(default = Duration::from_secs(30))]
    keep_alive_interval: Duration,
    #[builder(default = Duration::from_secs(10))]
    keep_alive_timeout: Duration,
    #[builder(default = 4 * 1024 * 1024)]
    max_decoding_message_size: usize,
}

impl Client {
    /// Establishes a connection to `orb-relay` and returns a client instance along with a join handle.
    /// The connection will automatically reconnect if lost, until the maximum number of connection attempts
    /// (configured via ClientOpts::max_connection_attempts) is reached.
    ///
    /// Messages sent while offline are buffered and eventually sent when connection is established.
    ///
    /// # Example
    /// ```ignore
    /// let opts = ClientOpts::entity()
    ///     .entity_type(EntityType::Orb)
    ///     .id("device_1")
    ///     .namespace("default")
    ///     .endpoint("https://relay.example.com")
    ///     .auth(Auth::Token("token123".to_string()))
    ///     .build();
    ///
    /// let (client, handle) = Client::connect(opts);
    /// ```
    pub fn connect(opts: ClientOpts) -> (Client, JoinHandle<Result<(), Err>>) {
        let (tonic_tx, tonic_rx) = flume::unbounded();
        let (client_tx, client_rx) = flume::unbounded();

        let props = actor::Props {
            client_tx,
            tonic_tx,
            tonic_rx,
            opts: opts.clone(),
        };

        let (actor_tx, join_handle) = actor::run(props);

        let client = Client {
            opts: Arc::new(opts),
            seq: Arc::new(AtomicU64::default()),
            actor_tx,
            client_rx,
        };

        (client, join_handle)
    }

    /// Stops the client from running. User can await the original join handle returned
    /// when client was created to wait for the client to finish stopping.
    ///
    /// # Example
    /// ```ignore
    /// let (client, handle) = Client::connect(opts);
    ///
    /// // Stop the client and wait for it to finish
    /// client.stop().await?;
    /// handle.await??;
    /// ```
    pub async fn stop(&self) -> Result<(), Err> {
        self.actor_tx
            .send(actor::Msg::Stop)
            .wrap_err("actor_tx failed to send Stop message")?;

        Ok(())
    }

    /// Forces the client to reconnect.
    ///
    /// # Example
    /// ```ignore
    /// let (client, handle) = Client::connect(opts);
    ///
    /// client.reconnect().await?;
    /// ```
    pub async fn reconnect(&self) -> Result<(), Err> {
        self.actor_tx
            .send(actor::Msg::Reconnect)
            .wrap_err("actor_tx failed to send Reconnect message")?;

        Ok(())
    }

    /// Asynchronously receives a message, yielding to the tokio runtime.
    /// Clone the client and pass it to a spawned task to avoid blocking.
    ///
    /// # Example
    /// ```ignore
    /// let client_rx = client.clone();
    /// tokio::spawn(async move {
    ///     while let Ok(msg) = client_rx.recv().await {
    ///         // Handle message
    ///     }
    /// });
    /// ```
    pub async fn recv(&self) -> Result<RecvMessage, Err> {
        let msg = self.client_rx.recv_async().await?;

        Ok(RecvMessage {
            from: msg.from,
            payload: msg.payload,
            type_url: msg.type_url,
            to: msg.to,
            client: self.clone(),
            seq: msg.seq,
        })
    }

    /// Sends a message without confirmation the other client received it, but optionally with confirmation
    /// that the orb-relay server received it by using `QoS::AtLeastOnce`
    ///
    /// # Example
    /// ```ignore
    /// // Send a message with at-most-once delivery (no ack)
    /// client.send(
    ///     SendMessage::to(EntityType::Orb)
    ///         .id("device_1")
    ///         .namespace("default")
    ///         .qos(QoS::AtMostOnce)
    ///         .payload(b"hello")
    /// ).await?;
    ///
    /// // Send with at-least-once delivery (waits for ack)
    /// client.send(
    ///     SendMessage::to(EntityType::Orb)
    ///         .id("device_1")
    ///         .namespace("default")
    ///         .qos(QoS::AtLeastOnce)
    ///         .payload(vec![1,2,3])
    /// ).await?;
    /// ```
    pub async fn send(&self, msg: SendMessage) -> Result<(), Err> {
        if let QoS::AtLeastOnce = msg.qos {
            self.send_with_ack(msg).await?;
        } else {
            let seq = self.seq.fetch_add(1, Ordering::SeqCst);
            let payload = relay_payload(&self.opts, &msg, seq);
            self.actor_tx
                .send(actor::Msg::Send(payload.into()))
                .wrap_err("Error reaching actor loop")?;
        }
        Ok(())
    }

    /// Wait for the relay ACK, including its subscriber count.
    /// This does not confirm receipt by the destination application.
    /// AtMostOnce waits for an ACK but never retransmits the message.
    pub async fn send_with_ack(&self, msg: SendMessage) -> Result<Ack, Err> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        let (ack_tx, ack_rx) = flume::unbounded();
        self.actor_tx
            .send(actor::Msg::WaitForAck {
                original_msg: msg,
                ack_tx,
                seq,
            })
            .wrap_err("Error reaching actor loop")?;
        ack_rx
            .recv_async()
            .await
            .wrap_err("Error when waiting for ack")?
    }

    /// Sends a message and waits for a reply from the destination client.
    /// Unlike `send()` with `QoS::AtLeastOnce` which only confirms delivery to the orb-relay server,
    /// this method waits for an actual response from the target client.
    ///
    /// # Example
    /// ```ignore
    /// // Send message and wait for response
    /// let response = client.ask(
    ///     SendMessage::to(EntityType::Orb)
    ///         .id("device_1")
    ///         .namespace("default")
    ///         .payload("what is your status?")
    /// ).await?;
    ///
    /// println!("Got response: {:?}", response);
    /// ```
    pub async fn ask(&self, msg: SendMessage) -> Result<Vec<u8>, Err> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        let (reply_tx, reply_rx) = flume::unbounded();

        self.actor_tx
            .send(actor::Msg::WaitForReply {
                original_msg: msg,
                recv_msg_tx: reply_tx,
                seq,
            })
            .wrap_err("Error reaching actor loop")?;

        let reply = reply_rx
            .recv_async()
            .await
            .wrap_err("Error when waiting for reply")??;

        Ok(reply.payload)
    }
}

pub(crate) fn relay_payload(
    opts: &ClientOpts,
    msg: &SendMessage,
    seq: Seq,
) -> RelayPayload {
    RelayPayload {
        src: Some(Entity {
            id: opts.client_id.clone(),
            entity_type: opts.entity_type as i32,
            namespace: opts.namespace.clone(),
        }),
        dst: Some(Entity {
            id: msg.target_id.clone(),
            entity_type: msg.target_type as i32,
            namespace: msg.target_namespace.clone(),
        }),
        seq,
        payload: Some(Any {
            type_url: msg.type_url.clone(),
            value: msg.data.clone(),
        }),
    }
}
