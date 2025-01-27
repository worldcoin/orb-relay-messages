use bon::Builder;
use color_eyre::eyre::{self, Context};
use derive_more::From;
use orb_relay_messages::prost_types::Any;
use orb_relay_messages::relay::{entity::EntityType, Entity, RelayPayload};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tonic::transport;

pub use amount::Amount;

mod actor;
mod amount;
mod flume_receiver_stream;

pub type ClientId = String;
pub type Seq = u64;

#[derive(Debug, Builder, Clone)]
#[builder(on(String, into))]
#[builder(on(Vec<u8>, into))]
#[builder(start_fn = to)]
#[builder(finish_fn = payload)]
pub struct SendMessage {
    #[builder(start_fn)]
    target_type: EntityType,
    #[builder(finish_fn)]
    data: Vec<u8>,
    #[builder(setters(name = id))]
    target_id: String,
    #[builder(setters(name = namespace))]
    target_namespace: String,
    #[builder(default = QoS::AtMostOnce)]
    qos: QoS,
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
    pub seq: Seq,
}

#[derive(Debug)]
pub struct RecvMessage<'a> {
    pub from: Entity,
    pub payload: Vec<u8>,
    client: &'a Client,
    seq: Seq,
}

impl<'a> RecvMessage<'a> {
    pub fn new(from: Entity, payload: Vec<u8>, client: &'a Client, seq: Seq) -> Self {
        Self {
            from,
            payload,
            client,
            seq,
        }
    }

    /// Reply to the sender of this message with a payload and specified QoS level.
    ///
    /// # Example
    /// ```ignore
    /// async fn handle_message(client: &Client) {
    ///     while let Ok(msg) = client.recv().await {
    ///         // Reply with "hello" using at-most-once delivery
    ///         msg.reply("hello", QoS::AtMostOnce).await.unwrap();
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
        let payload = payload.into();
        let relay_payload = RelayPayload {
            src: Some(Entity {
                id: self.client.opts.client_id.clone(),
                entity_type: self.client.opts.entity_type as i32,
                namespace: self.client.opts.namespace.clone(),
            }),
            dst: Some(Entity {
                id: self.from.id.clone(),
                entity_type: self.from.entity_type as i32,
                namespace: self.from.namespace.clone(),
            }),
            seq: self.seq,
            payload: Some(Any {
                type_url: "".to_string(),
                value: payload.clone(),
            }),
        };

        self.client
            .actor_tx
            .send(actor::Msg::Send(relay_payload.into()))
            .wrap_err("Failed to send message to tonic")?;

        if let QoS::AtLeastOnce = qos {
            let (ack_tx, ack_rx) = flume::unbounded();

            self.client
                .actor_tx
                .send(actor::Msg::WaitForAck {
                    original_msg: SendMessage {
                        target_type: EntityType::try_from(self.from.entity_type)
                            .wrap_err("Failed to convert EntityType")?,
                        data: payload,
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
                .wrap_err("Error when waiting for ack")?;
        };

        Ok(())
    }
}

#[derive(From, Debug)]
pub enum Err {
    StopRequest,
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
///     .endpoint("http://localhost:8080")
///     .auth_token("token123")
///     .build();
///
/// let (client, handle) = Client::connect(opts);
/// ```
pub struct Client {
    opts: Arc<ClientOpts>,
    seq: Arc<AtomicU64>,
    client_rx: flume::Receiver<RecvdRelayPayload>,
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
///     .endpoint("http://localhost:8080")
///     .auth_token("token123")
///     // Optional fields (defaults provided):
///     .connection_timeout(Duration::from_secs(30))
///     .max_connection_attempts(Amount::Infinite)
///     .ack_timeout(Duration::from_secs(20))
///     .reply_timeout(Duration::from_secs(20))
///     .heartbeat(Duration::from_secs(30))
///     .max_message_attempts(Amount::Infinite)
///     .build();
/// ```
#[derive(Debug, Builder, Clone)]
#[builder(on(String, into))]
#[builder(start_fn = entity)]
pub struct ClientOpts {
    #[builder(start_fn)]
    entity_type: EntityType,
    #[builder(setters(name = id))]
    client_id: String,
    namespace: String,
    endpoint: String,
    auth_token: String, // TODO: secrecy
    #[builder(default = Duration::from_secs(20))]
    connection_timeout: Duration,
    #[builder(default = Amount::Infinite)]
    max_connection_attempts: Amount,
    #[builder(default = Duration::from_secs(20))]
    ack_timeout: Duration,
    #[builder(default = Duration::from_secs(20))]
    reply_timeout: Duration,
    #[builder(default = Duration::from_secs(30))]
    heartbeat: Duration,
    #[builder(default = Amount::Infinite)]
    max_message_attempts: Amount,
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
    ///     .endpoint("http://localhost:8080")
    ///     .auth_token("token123")
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
    pub async fn recv(&self) -> Result<RecvMessage<'_>, Err> {
        let msg = self.client_rx.recv_async().await?;

        Ok(RecvMessage {
            from: msg.from,
            payload: msg.payload,
            client: self,
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
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        let payload = relay_payload(&self.opts, &msg, seq);

        self.actor_tx
            .send(actor::Msg::Send(payload.into()))
            .wrap_err("Failed to send message to tonic")?;

        if let QoS::AtLeastOnce = msg.qos {
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
                .wrap_err("Error when waiting for ack")?;
        };

        Ok(())
    }

    /// Sends a message and waits for a reply from the destination client.
    /// Unlike `send()` with `QoS::AtLeastOnce` which only confirms delivery to the orb-relay server,
    /// this method waits for an actual response from the target client.
    ///
    /// # Example
    /// ```ignore
    /// let response = client.ask(
    ///     SendMessage::to(EntityType::Orb)
    ///         .id("device_1")
    ///         .namespace("default")
    ///         .payload("what is your status?")
    /// ).await?;
    /// ```
    pub async fn ask(&self, msg: SendMessage) -> Result<Vec<u8>, Err> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        let payload = relay_payload(&self.opts, &msg, seq);

        self.actor_tx
            .send(actor::Msg::Send(payload.into()))
            .wrap_err("Failed to send message to tonic")?;

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
            .wrap_err("Error when waiting for ack")?;

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
            type_url: "".to_string(),
            value: msg.data.clone(),
        }),
    }
}
