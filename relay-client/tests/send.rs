/*
* Tests whether a client A can send a message to client B
*/
use orb_relay_client::{
    Amount, Auth, Client, ClientOpts, QoS, RelayClient, SendMessage,
};
use orb_relay_messages::{
    prost_types::Any,
    relay::{
        entity::EntityType, relay_connect_request::Msg, Ack, ConnectRequest,
        ConnectResponse, Entity, RelayPayload,
    },
};
use orb_relay_test_utils::{IntoRes, TestServer};
use std::time::Duration;
use tokio::time;

#[tokio::test]
async fn sends_at_most_once_and_increases_seq() {
    // Arrange
    let sv = TestServer::new(vec![], |state, conn_req, _| match conn_req {
        Msg::ConnectRequest(ConnectRequest { client_id, .. }) => ConnectResponse {
            client_id: client_id.unwrap().id.clone(),
            success: true,
            error: "nothing".to_string(),
        }
        .into_res(),

        _ => {
            state.push(conn_req.clone());
            None
        }
    })
    .await;

    let opts = ClientOpts::entity(EntityType::App)
        .id("foo")
        .namespace("bar")
        .endpoint(format!("http://{}", sv.addr()))
        .auth(Auth::Token(Default::default()))
        .max_connection_attempts(Amount::Val(1))
        .connection_timeout(Duration::from_millis(10))
        .heartbeat(Duration::from_secs(u64::MAX))
        .ack_timeout(Duration::from_millis(1)) // so we can test that we aren't retrying when QoS is AtMostOnce
        .build();

    let (client, _handle) = Client::connect(opts);

    // Act
    let msg = SendMessage::to(EntityType::Orb)
        .id("thomas")
        .namespace("anderson")
        .qos(QoS::AtMostOnce)
        .payload(b"woah");

    client.send(msg).await.unwrap();

    let msg = SendMessage::to(EntityType::Orb)
        .id("thomas")
        .namespace("anderson")
        .qos(QoS::AtMostOnce)
        .payload(b"i know kung-fu");

    client.send(msg).await.unwrap();

    // Assert
    time::sleep(Duration::from_millis(50)).await;
    let actual = sv.state().await.clone();
    let src = Some(Entity {
        id: "foo".to_string(),
        entity_type: EntityType::App as i32,
        namespace: "bar".to_string(),
    });

    let dst = Some(Entity {
        id: "thomas".to_string(),
        entity_type: EntityType::Orb as i32,
        namespace: "anderson".to_string(),
    });

    let expected = [
        Msg::Payload(orb_relay_messages::relay::RelayPayload {
            src: src.clone(),
            dst: dst.clone(),
            seq: 0,
            payload: Some(Any {
                type_url: String::default(),
                value: b"woah".to_vec(),
            }),
        }),
        Msg::Payload(orb_relay_messages::relay::RelayPayload {
            src,
            dst,
            seq: 1,
            payload: Some(Any {
                type_url: String::default(),
                value: b"i know kung-fu".to_vec(),
            }),
        }),
    ];

    assert_eq!(actual, expected)
}

#[tokio::test]
async fn sends_at_least_once_retrying_until_ack_is_received() {
    // Arrange
    let max_attempts = 3;
    let sv = TestServer::new(vec![], move |state, conn_req, _| match conn_req {
        Msg::ConnectRequest(ConnectRequest { client_id, .. }) => ConnectResponse {
            client_id: client_id.unwrap().id.clone(),
            success: true,
            error: "nothing".to_string(),
        }
        .into_res(),

        Msg::Payload(RelayPayload { seq, .. }) => {
            state.push(conn_req.clone());

            if state.len() >= max_attempts {
                Ack { seq }.into_res()
            } else {
                None
            }
        }

        _ => None,
    })
    .await;

    let opts = ClientOpts::entity(EntityType::App)
        .id("foo")
        .namespace("bar")
        .endpoint(format!("http://{}", sv.addr()))
        .auth(Auth::Token(Default::default()))
        .max_connection_attempts(Amount::Val(1))
        .connection_timeout(Duration::from_millis(10))
        .heartbeat(Duration::from_secs(u64::MAX))
        .ack_timeout(Duration::from_millis(10))
        .build();

    let (client, _handle) = Client::connect(opts);

    // Act
    let msg = SendMessage::to(EntityType::Orb)
        .id("thomas")
        .namespace("anderson")
        .qos(QoS::AtLeastOnce)
        .payload(b"woah");

    client.send(msg).await.unwrap();

    // Assert
    time::sleep(Duration::from_millis(100)).await; // wait extra time so we see if we send any extra attempts
    let actual = sv.state().await.clone();

    let msg = Msg::Payload(orb_relay_messages::relay::RelayPayload {
        src: Some(Entity {
            id: "foo".to_string(),
            entity_type: EntityType::App as i32,
            namespace: "bar".to_string(),
        }),
        dst: Some(Entity {
            id: "thomas".to_string(),
            entity_type: EntityType::Orb as i32,
            namespace: "anderson".to_string(),
        }),
        seq: 0,
        payload: Some(Any {
            type_url: String::default(),
            value: b"woah".to_vec(),
        }),
    });

    let expected = [msg.clone(), msg.clone(), msg.clone()];

    assert_eq!(actual.len(), expected.len());
    assert_eq!(actual, expected)
}
