use bon::Builder;
use orb_relay_messages::relay::{entity::EntityType, Entity};
use std::time::Duration;
use tokio::task;

mod flume_receiver_stream;
mod receiver;

pub type ClientId = String;
pub type Seq = u64;

#[derive(Builder)]
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
pub enum QoS {
    AtMostOnce,
    ExactlyOnce { max_retries: u64, backoff: Duration },
}

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

pub struct Receiver;
impl Receiver {
    pub async fn recv(&self) -> Result<RecvMessage, RecvErr> {
        todo!()
    }
}

#[derive(Debug)]
pub enum ClientErr {
    Todo,
}

pub struct Client;

impl Client {
    pub async fn connect(
        entity_type: EntityType,
        id: &str,
        namespace: &str,
    ) -> Result<(Client, Receiver), ClientErr> {
        todo!()
    }

    pub async fn send(&self, msg: SendMessage) {}
    pub async fn ask(&self, msg: SendMessage) -> Result<RecvMessage, RecvErr> {
        todo!()
    }
}

async fn test() {
    // will have reconnect options, retry configuration etc
    let (client, client_rx) = Client::connect(EntityType::App, "123", "cool-namespace")
        .await
        .unwrap();

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
        .qos(QoS::ExactlyOnce)
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
