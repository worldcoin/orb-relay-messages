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

fn options(endpoint: String, retries: u64) -> ClientOpts {
    ClientOpts::entity(EntityType::Orb)
        .id("orb")
        .namespace("test")
        .endpoint(endpoint)
        .auth(Auth::Token(Default::default()))
        .max_connection_attempts(Amount::Val(1))
        .max_message_attempts(Amount::Val(retries))
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
        .qos(qos)
        .payload(Any {
            type_url: "type.googleapis.com/test.Message".into(),
            value: b"synthetic-payload".to_vec(),
        })
}

#[tokio::test]
async fn receive_and_acknowledged_reply_preserve_type() {
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
                if packets.len() == 1 {
                    return None;
                }
                Ack {
                    seq,
                    subscriber_count: Some(1),
                }
                .into_res()
            }
            _ => None,
        })
        .await;
    let (client, task) =
        Client::connect(options(format!("https://{}", server.addr()), 2));
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
        received.reply(reply.clone(), QoS::AtLeastOnce),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(Some(received.payload), expected.payload);
    assert_eq!(Some(received.from), expected.src);
    let packets = server.state().await;
    assert_eq!(packets.len(), 2);
    assert_eq!(packets[0], packets[1]);
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
    for (ack, qos, retries, expected_attempts) in [
        (true, QoS::AtLeastOnce, 2, 3),
        (true, QoS::AtLeastOnce, 0, 1),
        (false, QoS::AtLeastOnce, 2, 3),
        (false, QoS::AtMostOnce, 2, 1),
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
            Client::connect(options(format!("https://{}", server.addr()), retries));
        let error = timeout(Duration::from_secs(3), async {
            if ack {
                client.send(message(qos)).await.unwrap_err()
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
async fn connection_failures_remain_distinguishable() {
    for mode in 0..3 {
        let server = TestServer::new((), move |_, request, clients| {
            let Msg::ConnectRequest(request) = request else {
                return None;
            };
            match mode {
                0 => ConnectResponse {
                    success: false,
                    ..Default::default()
                }
                .into_res(),
                1 => None,
                _ => {
                    clients.stop(&request.client_id.unwrap());
                    None
                }
            }
        })
        .await;
        let (_client, task) =
            Client::connect(options(format!("https://{}", server.addr()), 2));
        let error = timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(matches!(
            (mode, error),
            (0, Err::AuthenticationFailed)
                | (1, Err::ConnectionTimeout)
                | (2, Err::StreamEnded)
        ));
    }
}
