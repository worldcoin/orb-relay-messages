use bon::Builder;
use color_eyre::eyre::{self, Context};
use derive_more::From;
use orb_relay_messages::prost_types::Any;
use orb_relay_messages::relay::RelayConnectRequest;
use orb_relay_messages::relay::{entity::EntityType, Entity, RelayPayload};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::{self, JoinHandle};
use tonic::transport;

mod actor;
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

/// Guarantee of delivery to `orb-relay`
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
            .tonic_tx
            .send(relay_payload.into())
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
pub struct Client {
    opts: Arc<ClientOpts>,
    seq: Arc<AtomicU64>,
    client_rx: flume::Receiver<RecvdRelayPayload>,
    tonic_tx: flume::Sender<RelayConnectRequest>,
    actor_tx: flume::Sender<actor::Msg>,
}

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
    heartbeat_secs: Duration,
    #[builder(default = Amount::Infinite)]
    max_message_attempts: Amount,
}

impl Client {
    pub fn connect(opts: ClientOpts) -> (Client, JoinHandle<Result<(), Err>>) {
        let (tonic_tx, tonic_rx) = flume::unbounded();
        let (client_tx, client_rx) = flume::unbounded();

        let props = actor::Props {
            client_tx,
            tonic_tx: tonic_tx.clone(),
            tonic_rx,
            opts: opts.clone(),
        };

        let (actor_tx, join_handle) = actor::run(props);

        let client = Client {
            opts: Arc::new(opts),
            seq: Arc::new(AtomicU64::default()),
            actor_tx,
            tonic_tx,
            client_rx,
        };

        (client, join_handle)
    }

    pub async fn stop(&self) {
        todo!()
    }

    pub async fn recv(&self) -> Result<RecvMessage<'_>, Err> {
        let msg = self.client_rx.recv_async().await?;

        Ok(RecvMessage {
            from: msg.from,
            payload: msg.payload,
            client: self,
            seq: msg.seq,
        })
    }

    pub async fn send(&self, msg: SendMessage) -> Result<(), Err> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        let payload = relay_payload(&self.opts, &msg, seq);

        self.tonic_tx
            .send(payload.into())
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

    pub async fn ask(&self, msg: SendMessage) -> Result<Vec<u8>, Err> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        let payload = relay_payload(&self.opts, &msg, seq);

        self.tonic_tx
            .send(payload.into())
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

#[derive(Debug, Copy, Clone)]
pub enum Amount {
    Val(u64),
    Infinite,
}

impl PartialEq<u64> for Amount {
    fn eq(&self, other: &u64) -> bool {
        match self {
            Amount::Val(v) => v == other,
            Amount::Infinite => false,
        }
    }
}

impl PartialOrd<u64> for Amount {
    fn partial_cmp(&self, other: &u64) -> Option<std::cmp::Ordering> {
        match self {
            Amount::Val(v) => v.partial_cmp(other),
            Amount::Infinite => Some(std::cmp::Ordering::Greater),
        }
    }
}

impl Amount {
    pub fn is_nonzero(&self) -> bool {
        if let Amount::Val(0) = self {
            return false;
        };

        true
    }
}

impl Amount {
    pub(crate) fn decrement(self) -> Self {
        match self {
            Amount::Val(v) if v > 0 => Amount::Val(v - 1),
            _ => self,
        }
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
