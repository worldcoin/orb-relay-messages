use bon::Builder;
use orb_relay_messages::relay::{entity::EntityType, Entity, RelayPayload};
use std::{cmp::Ordering, time::Duration};
use tokio::task::{self, JoinHandle};

mod flume_receiver_stream;
mod receiver;

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

#[derive(Debug)]
pub enum RecvErr {
    StreamEnded,
}

pub struct Receiver {
    client_rx: flume::Receiver<RecvMessage>,
}

impl Receiver {
    pub async fn recv(&self) -> Result<RecvMessage, RecvErr> {
        todo!()
    }
}

#[derive(Debug)]
pub enum ClientErr {
    Todo,
}

#[derive(Debug, Builder)]
#[builder(on(String, into))]
pub struct Client {
    opts: ClientOpts,
    seq: Seq,
    client_rx: flume::Receiver<RecvMessage>,
    actor_tx: flume::Sender<receiver::Msg>,
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

        let props = receiver::Props {
            client_tx,
            tonic_tx,
            tonic_rx,
            opts: opts.clone(),
        };

        let (actor_tx, join_handle) = receiver::run(props);

        let client = Client {
            opts,
            seq: 0,
            actor_tx,
            client_rx,
        };

        (client, join_handle)
    }

    pub fn receiver(&self) -> Receiver {
        Receiver {
            client_rx: self.client_rx.clone(),
        }
    }

    pub async fn send(&self, msg: SendMessage) {}
    pub async fn ask(&self, msg: SendMessage) -> Result<RecvMessage, RecvErr> {
        todo!()
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

#[derive(Debug)]
pub struct RetryStrategy {
    max_retries: u64,
    timeout: Duration,
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
