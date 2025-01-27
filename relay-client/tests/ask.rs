/*
* Tests whether a client A can ask a message from client B
*/
mod test_server;

use orb_relay_messages::{
    prost_types::Any,
    relay::{
        entity::EntityType, relay_connect_request::Msg, ConnectRequest,
        ConnectResponse, Entity, RelayPayload,
    },
};
use relay_client::{Amount, Client, ClientOpts, QoS, SendMessage};
use std::{time::Duration, u64};
use test_server::{IntoRes, TestServer};
use tokio::time;

#[tokio::test]
async fn asks_at_most_once() {
    // Arrange
    let sv = TestServer::new(vec![], move |state, conn_req| match &conn_req {
        Msg::ConnectRequest(ConnectRequest { client_id, .. }) => ConnectResponse {
            client_id: client_id.as_ref().unwrap().id.clone(),
            success: true,
            error: "nothing".to_string(),
        }
        .into_res(),

        Msg::Payload(RelayPayload { src, dst, seq, .. }) => {
            state.push(conn_req.clone());

            // pretend client receiving msg has replied
            RelayPayload {
                src: dst.clone(),
                dst: src.clone(),
                seq: *seq,
                payload: Some(Any {
                    type_url: String::default(),
                    value: b"pong".to_vec(),
                }),
            }
            .into_res()
        }

        other => panic!("did not expect {other:?}"),
    })
    .await;

    let opts = ClientOpts::entity(EntityType::App)
        .id("apple")
        .namespace("green")
        .endpoint(format!("http://{}", sv.addr()))
        .auth_token(String::default())
        .max_connection_attempts(Amount::Val(1))
        .connection_timeout(Duration::from_millis(10))
        .heartbeat(Duration::from_secs(u64::MAX))
        .reply_timeout(Duration::from_millis(1)) // so we can test that we aren't retrying when QoS is AtMostOnce
        .build();

    let (client, _handle) = Client::connect(opts);
    time::sleep(Duration::from_millis(50)).await;

    // Act
    let msg = SendMessage::to(EntityType::Orb)
        .id("banana")
        .namespace("yellow")
        .qos(QoS::AtMostOnce)
        .payload(b"ping");

    let reply = client.ask(msg).await.unwrap();

    // Assert
    assert_eq!(b"pong".as_ref(), reply);

    // we await 50ms because if there is a bug given reply timeout is 1ms we would see multiple messages
    time::sleep(Duration::from_millis(50)).await;
    let actual = sv.state().await.clone();

    let expected = [Msg::Payload(orb_relay_messages::relay::RelayPayload {
        src: Some(Entity {
            id: "apple".to_string(),
            entity_type: EntityType::App as i32,
            namespace: "green".to_string(),
        }),
        dst: Some(Entity {
            id: "banana".to_string(),
            entity_type: EntityType::Orb as i32,
            namespace: "yellow".to_string(),
        }),
        seq: 0,
        payload: Some(Any {
            type_url: String::default(),
            value: b"ping".to_vec(),
        }),
    })];

    assert_eq!(actual, expected)
}

#[tokio::test]
async fn ask_sends_at_least_once_retrying_until_reply_is_received() {
    // Arrange
    let max_attempts = 3;
    let sv = TestServer::new(vec![], move |state, conn_req| match &conn_req {
        Msg::ConnectRequest(ConnectRequest { client_id, .. }) => ConnectResponse {
            client_id: client_id.as_ref().unwrap().id.clone(),
            success: true,
            error: "nothing".to_string(),
        }
        .into_res(),

        Msg::Payload(RelayPayload { src, dst, seq, .. }) => {
            state.push(conn_req.clone());

            if state.len() >= max_attempts {
                RelayPayload {
                    src: dst.clone(),
                    dst: src.clone(),
                    seq: *seq,
                    payload: Some(Any {
                        type_url: String::default(),
                        value: b"pong".to_vec(),
                    }),
                }
                .into_res()
            } else {
                None
            }
        }

        _ => None,
    })
    .await;

    let opts = ClientOpts::entity(EntityType::App)
        .id("papaya")
        .namespace("orange")
        .endpoint(format!("http://{}", sv.addr()))
        .auth_token(String::default())
        .max_connection_attempts(Amount::Val(1))
        .connection_timeout(Duration::from_millis(10))
        .heartbeat(Duration::from_secs(u64::MAX))
        .reply_timeout(Duration::from_millis(10))
        .build();

    let (client, _handle) = Client::connect(opts);

    // Act
    let msg = SendMessage::to(EntityType::Orb)
        .id("blueberry")
        .namespace("purple")
        .qos(QoS::AtLeastOnce)
        .payload(b"ping");

    let reply = client.ask(msg).await.unwrap();

    // Assert
    assert_eq!(b"pong".as_ref(), reply);

    time::sleep(Duration::from_millis(100)).await; // wait extra time so we see if we send any extra attempts
    let actual = sv.state().await.clone();

    let msg = Msg::Payload(orb_relay_messages::relay::RelayPayload {
        src: Some(Entity {
            id: "papaya".to_string(),
            entity_type: EntityType::App as i32,
            namespace: "orange".to_string(),
        }),
        dst: Some(Entity {
            id: "blueberry".to_string(),
            entity_type: EntityType::Orb as i32,
            namespace: "purple".to_string(),
        }),
        seq: 0,
        payload: Some(Any {
            type_url: String::default(),
            value: b"ping".to_vec(),
        }),
    });

    let expected = [msg.clone(), msg.clone(), msg.clone()];

    assert_eq!(actual.len(), expected.len());
    assert_eq!(actual, expected)
}

#[tokio::test]
async fn ask_increases_seq() {
    // Arrange
    let sv = TestServer::new(vec![], move |state, conn_req| match &conn_req {
        Msg::ConnectRequest(ConnectRequest { client_id, .. }) => ConnectResponse {
            client_id: client_id.as_ref().unwrap().id.clone(),
            success: true,
            error: "nothing".to_string(),
        }
        .into_res(),

        Msg::Payload(RelayPayload { src, dst, seq, .. }) => {
            state.push(conn_req.clone());

            // pretend client receiving msg has replied
            RelayPayload {
                src: dst.clone(),
                dst: src.clone(),
                seq: *seq,
                payload: Some(Any {
                    type_url: String::default(),
                    value: b"pong".to_vec(),
                }),
            }
            .into_res()
        }

        other => panic!("did not expect {other:?}"),
    })
    .await;

    let opts = ClientOpts::entity(EntityType::App)
        .id("apple")
        .namespace("green")
        .endpoint(format!("http://{}", sv.addr()))
        .auth_token(String::default())
        .max_connection_attempts(Amount::Val(1))
        .connection_timeout(Duration::from_millis(10))
        .heartbeat(Duration::from_secs(u64::MAX))
        .reply_timeout(Duration::from_millis(1)) // so we can test that we aren't retrying when QoS is AtMostOnce
        .build();

    let (client, _handle) = Client::connect(opts);
    time::sleep(Duration::from_millis(50)).await;

    // Act
    let msg = SendMessage::to(EntityType::Orb)
        .id("banana")
        .namespace("yellow")
        .qos(QoS::AtMostOnce)
        .payload(b"ping");

    let reply1 = client.ask(msg.clone()).await.unwrap();
    let reply2 = client.ask(msg.clone()).await.unwrap();
    let reply3 = client.ask(msg).await.unwrap();

    // Assert
    assert_eq!(b"pong".as_ref(), reply1);
    assert_eq!(b"pong".as_ref(), reply2);
    assert_eq!(b"pong".as_ref(), reply3);

    // we await 50ms because if there is a bug given reply timeout is 1ms we would see multiple messages
    time::sleep(Duration::from_millis(50)).await;
    let actual = sv.state().await.clone();
    let msg = |seq| {
        Msg::Payload(orb_relay_messages::relay::RelayPayload {
            src: Some(Entity {
                id: "apple".to_string(),
                entity_type: EntityType::App as i32,
                namespace: "green".to_string(),
            }),
            dst: Some(Entity {
                id: "banana".to_string(),
                entity_type: EntityType::Orb as i32,
                namespace: "yellow".to_string(),
            }),
            seq,
            payload: Some(Any {
                type_url: String::default(),
                value: b"ping".to_vec(),
            }),
        })
    };

    let expected = [msg(0), msg(1), msg(2)];

    assert_eq!(actual, expected)
}
