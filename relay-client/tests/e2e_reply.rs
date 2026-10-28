#![cfg(feature = "testing")]
use orb_relay_client::{Amount, Auth, Client, ClientOpts, QoS, SendMessage};
use orb_relay_messages::{
    prost_types::Any,
    relay::{
        entity::EntityType, relay_connect_request::Msg, Ack, ConnectRequest,
        ConnectResponse, Heartbeat,
    },
};
use orb_relay_test_utils::{IntoRes, TestServer};
use std::time::Duration;
use tokio::task;

struct NoState;

#[tokio::test]
async fn sends_and_replies_to_message() {
    // Arrange
    let sv = TestServer::new(NoState, |_, conn_req, clients| match conn_req {
        Msg::ConnectRequest(ConnectRequest { client_id, .. }) => ConnectResponse {
            client_id: client_id.unwrap().id.clone(),
            success: true,
            error: "nothing".to_string(),
        }
        .into_res(),

        Msg::Payload(payload) => {
            clients.send(payload);
            None
        }

        _ => None,
    })
    .await;

    let opts_foo = ClientOpts::entity(EntityType::App)
        .id("foo")
        .namespace("bar")
        .endpoint(format!("https://{}", sv.addr()))
        .auth(Auth::Token(Default::default()))
        .max_connection_attempts(Amount::Val(1))
        .connection_timeout(Duration::from_secs(5))
        .heartbeat(Duration::from_secs(u64::MAX))
        .ack_timeout(Duration::from_millis(1)) // so we can test that we aren't retrying when QoS is AtMostOnce
        .build();

    let opts_boo = ClientOpts::entity(EntityType::App)
        .id("boo")
        .namespace("bar")
        .endpoint(format!("https://{}", sv.addr()))
        .auth(Auth::Token(Default::default()))
        .max_connection_attempts(Amount::Val(1))
        .connection_timeout(Duration::from_secs(5))
        .heartbeat(Duration::from_secs(u64::MAX))
        .ack_timeout(Duration::from_millis(1)) // so we can test that we aren't retrying when QoS is AtMostOnce
        .build();

    let (client_foo, _) = Client::connect(opts_foo);
    let (client_boo, _) = Client::connect(opts_boo);

    let question = Any::from_msg(&Heartbeat { seq: 7 }).unwrap();
    let answer = Any::from_msg(&Ack {
        seq: 7,
        subscriber_count: Some(1),
    })
    .unwrap();

    // Act
    let msg = SendMessage::to(EntityType::App)
        .id("boo")
        .namespace("bar")
        .payload(question.clone());

    let reply = answer.clone();
    let received = task::spawn(async move {
        let msg = client_boo.recv().await.unwrap();
        msg.reply(reply, QoS::AtMostOnce).await.unwrap();
        msg.payload
    });

    let res = client_foo.ask(msg).await.unwrap();

    // Assert
    assert_eq!(received.await.unwrap(), question);
    assert_eq!(res, answer)
}
