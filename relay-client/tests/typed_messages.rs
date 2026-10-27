#![cfg(feature = "testing")]

use orb_relay_client::{Amount, Auth, Client, ClientOpts, Err, QoS, SendMessage};
use orb_relay_messages::{
    prost_types::Any,
    relay::{
        entity::EntityType, relay_connect_request::Msg, Ack, ConnectResponse, Entity,
        RelayPayload,
    },
};
use orb_relay_test_utils::{IntoRes, TestServer};
use std::time::Duration;
use tokio::time::timeout;

fn options(endpoint: String) -> ClientOpts {
    ClientOpts::entity(EntityType::Orb)
        .id("orb")
        .namespace("test")
        .endpoint(endpoint)
        .auth(Auth::Token(Default::default()))
        .max_connection_attempts(Amount::Val(1))
        .max_message_attempts(Amount::Val(2))
        .connection_timeout(Duration::from_secs(2))
        .ack_timeout(Duration::from_millis(50))
        .reply_timeout(Duration::from_millis(50))
        .heartbeat(Duration::from_millis(10))
        .build()
}

fn message(qos: QoS) -> SendMessage {
    SendMessage::to(EntityType::App)
        .id("app")
        .namespace("test")
        .type_url("type.googleapis.com/test.Message")
        .qos(qos)
        .payload(b"synthetic-payload")
}

#[tokio::test]
async fn typed_send_preserves_payload_and_ack_details() {
    for subscribers in [None, Some(0), Some(1)] {
        let server =
            TestServer::new(vec![], move |packets, request, _| match request {
                Msg::ConnectRequest(request) => ConnectResponse {
                    client_id: request.client_id.unwrap().id,
                    success: true,
                    ..Default::default()
                }
                .into_res(),
                Msg::Payload(packet) => {
                    let seq = packet.seq;
                    packets.push(packet);
                    Ack {
                        seq,
                        subscriber_count: subscribers,
                    }
                    .into_res()
                }
                _ => None,
            })
            .await;
        let (client, task) =
            Client::connect(options(format!("https://{}", server.addr())));
        let ack = timeout(
            Duration::from_secs(3),
            client.send_with_ack(message(QoS::AtMostOnce)),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(ack.subscriber_count, subscribers);
        let packets = server.state().await;
        assert_eq!(packets.len(), 1);
        assert_eq!(
            packets[0].payload.as_ref().unwrap().type_url,
            "type.googleapis.com/test.Message"
        );
        assert_eq!(
            packets[0].payload.as_ref().unwrap().value,
            b"synthetic-payload"
        );
        assert_eq!(packets[0].dst.as_ref().unwrap().id, "app");
        assert_eq!(packets[0].src.as_ref().unwrap().id, "orb");
        drop(packets);
        client.stop().await.unwrap();
        timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn receive_and_reply_preserve_typed_envelope() {
    let packet = RelayPayload {
        src: Some(Entity {
            id: "app".into(),
            entity_type: EntityType::App as i32,
            namespace: "test".into(),
        }),
        dst: Some(Entity {
            id: "orb".into(),
            entity_type: EntityType::Orb as i32,
            namespace: "test".into(),
        }),
        seq: 42,
        payload: Some(Any {
            type_url: "type.googleapis.com/test.Incoming".into(),
            value: vec![1, 2, 3],
        }),
    };
    let expected = packet.clone();
    let server =
        TestServer::new(vec![], move |packets, request, clients| match request {
            Msg::ConnectRequest(request) => {
                clients.send(packet.clone());
                ConnectResponse {
                    client_id: request.client_id.unwrap().id,
                    success: true,
                    ..Default::default()
                }
                .into_res()
            }
            Msg::Payload(packet) => {
                let seq = packet.seq;
                packets.push(packet);
                Ack {
                    seq,
                    subscriber_count: Some(1),
                }
                .into_res()
            }
            _ => None,
        })
        .await;
    let (client, task) = Client::connect(options(format!("https://{}", server.addr())));
    let received = timeout(Duration::from_secs(3), client.recv())
        .await
        .unwrap()
        .unwrap();
    let reply = Any {
        type_url: "type.googleapis.com/test.Reply".into(),
        value: vec![4, 5],
    };
    timeout(
        Duration::from_secs(3),
        received.reply_any(reply.clone(), QoS::AtLeastOnce),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(received.into_relay_payload(), expected);
    let packets = server.state().await;
    assert_eq!(packets[0].payload, Some(reply));
    assert_eq!(packets[0].seq, 42);
    assert_eq!(packets[0].dst, expected.src);
    drop(packets);
    client.stop().await.unwrap();
    timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn finite_retries_terminate_and_heartbeat_acks_do_not_acknowledge_payloads() {
    for (ack, qos, expected_attempts) in [
        (true, QoS::AtLeastOnce, 3),
        (false, QoS::AtLeastOnce, 3),
        (true, QoS::AtMostOnce, 1),
        (false, QoS::AtMostOnce, 1),
    ] {
        let server = TestServer::new(0, |attempts, request, _| match request {
            Msg::ConnectRequest(request) => ConnectResponse {
                client_id: request.client_id.unwrap().id,
                success: true,
                ..Default::default()
            }
            .into_res(),
            Msg::Payload(packet) => {
                assert_eq!(
                    packet.payload.unwrap().type_url,
                    "type.googleapis.com/test.Message"
                );
                *attempts += 1;
                None
            }
            Msg::Heartbeat(heartbeat) => Ack {
                seq: heartbeat.seq,
                subscriber_count: None,
            }
            .into_res(),
        })
        .await;
        let (client, task) =
            Client::connect(options(format!("https://{}", server.addr())));
        let error = timeout(Duration::from_secs(3), async {
            if ack {
                client.send_with_ack(message(qos)).await.unwrap_err()
            } else {
                client.ask(message(qos)).await.unwrap_err()
            }
        })
        .await
        .unwrap();
        assert!(matches!(
            (ack, error),
            (true, Err::AckTimeout) | (false, Err::ReplyTimeout)
        ));
        assert_eq!(*server.state().await, expected_attempts);
        client.stop().await.unwrap();
        timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn rejects_wrong_identity_and_payload_before_authentication() {
    for wrong_identity in [true, false] {
        let server = TestServer::new((), move |_, _, _| {
            if wrong_identity {
                ConnectResponse {
                    client_id: "different-orb".into(),
                    success: true,
                    ..Default::default()
                }
                .into_res()
            } else {
                RelayPayload::default().into_res()
            }
        })
        .await;
        let (_client, task) =
            Client::connect(options(format!("https://{}", server.addr())));
        let error = timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(matches!(
            (wrong_identity, error),
            (true, Err::AuthenticationFailed) | (false, Err::UnexpectedResponse)
        ));
    }
}
