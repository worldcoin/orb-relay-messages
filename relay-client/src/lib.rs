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

mod actor;
mod flume_receiver_stream;

pub type ClientId = String;
pub type Seq = u64;

#[derive(Debug, Builder)]
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

#[derive(Debug)]
pub struct RecvMessage {
    pub from: Entity,
    pub payload: Vec<u8>,
}

impl RecvMessage {
    pub fn new(from: Entity, payload: Vec<u8>) -> Self {
        Self { from, payload }
    }

    pub async fn reply(&self, payload: impl Into<Vec<u8>>) {}
}

#[derive(Debug, From)]
pub enum RecvErr {
    StreamEnded,
    Generic(eyre::Error),
}

pub struct Receiver {
    client_rx: flume::Receiver<RecvMessage>,
}

impl Receiver {
    pub async fn recv(&self) -> Result<RecvMessage, RecvErr> {
        todo!()
    }
}

#[derive(Debug, From)]
pub enum ClientErr {
    Generic(eyre::Error),
}

#[derive(Debug, Builder, Clone)]
#[builder(on(String, into))]
pub struct Client {
    opts: Arc<ClientOpts>,
    seq: Arc<AtomicU64>,
    client_rx: flume::Receiver<RecvMessage>,
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
    domain: String,
    auth_token: String, // TODO: secrecy
    #[builder(default = Duration::from_secs(20))]
    connection_timeout: Duration,
    #[builder(default = Amount::Infinite)]
    max_connection_attempts: Amount,
    #[builder(default = Duration::from_secs(20))]
    ack_timeout: Duration,
    #[builder(default = Duration::from_secs(20))]
    reply_timeout: Duration,
    #[builder(default = Amount::Infinite)]
    max_message_attempts: Amount,
}

impl Client {
    pub fn connect(opts: ClientOpts) -> (Client, JoinHandle<()>) {
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

    pub fn receiver(&self) -> Receiver {
        Receiver {
            client_rx: self.client_rx.clone(),
        }
    }

    pub async fn send(&self, msg: SendMessage) -> Result<(), ClientErr> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);

        let payload = RelayPayload {
            src: Some(Entity {
                id: self.opts.client_id.clone(),
                entity_type: self.opts.entity_type as i32,
                namespace: self.opts.namespace.clone(),
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
        };

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

    pub async fn ask(&self, msg: SendMessage) -> Result<RecvMessage, RecvErr> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);

        let payload = RelayPayload {
            src: Some(Entity {
                id: self.opts.client_id.clone(),
                entity_type: self.opts.entity_type as i32,
                namespace: self.opts.namespace.clone(),
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
        };

        self.tonic_tx
            .send(payload.into())
            .wrap_err("Failed to send message to tonic")?;

        let (reply_tx, reply_rx) = flume::unbounded();

        self.actor_tx
            .send(actor::Msg::WaitForReply {
                from_client: msg.target_id.clone(),
                seq,
                recv_msg_tx: reply_tx,
                qos: msg.qos,
            })
            .wrap_err("Error reaching actor loop")?;

        let reply = reply_rx
            .recv_async()
            .await
            .wrap_err("Error when waiting for ack")?;

        Ok(reply)
    }
}

#[derive(Debug, Copy, Clone)]
pub enum Amount {
    Val(u64),
    Infinite,
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

async fn test() {
    // will have reconnect options, retry configuration etc
    let opts = ClientOpts::entity(EntityType::App)
        .id("id")
        .namespace("namespace")
        .auth_token("secret")
        .domain("dadsa.com")
        .build();

    let (client, _handle) = Client::connect(opts);
    let client_rx = client.receiver();

    // sending a message without ack form orb-relay
    let msg = SendMessage::to(EntityType::App)
        .id("123")
        .namespace("cool-namespace")
        .qos(QoS::AtMostOnce) // this is optional, and default value is QoS::AtMostOnce
        .payload(b"hi");

    client.send(msg).await;

    // sending a message waiting for an ack form orb-relay
    let msg = SendMessage::to(EntityType::App)
        .id("123")
        .namespace("cool-namespace")
        .qos(QoS::AtLeastOnce)
        .payload(b"hi");

    client.send(msg).await;

    // spawning a thread to listen ot messages
    task::spawn(async move {
        while let Ok(msg) = client_rx.recv().await {
            if msg.from.id == "123" {
                let payload = String::from_utf8_lossy(&msg.payload);

                if payload == "what is your name" {
                    msg.reply(b"am i talking to myself").await;
                } else {
                    println!("got {payload}");
                }
            }
        }
    });

    // sending an ask message, where we can get a response
    let msg = SendMessage::to(EntityType::App)
        .id("123")
        .namespace("cool-namespace")
        .payload(b"what is your name");

    let res = client.ask(msg).await.unwrap();
    let res_msg = String::from_utf8_lossy(&res.payload);
    println!("got {res_msg} from {}", res.from.id);
}
