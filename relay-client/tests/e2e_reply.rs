#![cfg(feature = "testing")]
use orb_relay_client::{Amount, Auth, Client, ClientOpts, SendMessage};
use orb_relay_messages::relay::{
    entity::EntityType, relay_connect_request::Msg, ConnectRequest, ConnectResponse,
};
use orb_relay_test_utils::{ConnectedClients, IntoRes, TestServer};
use std::time::Duration;
use tokio::{task, time::timeout};
use tonic::{Request, Status};

struct NoState;

#[tokio::test]
async fn sends_and_replies_to_message() {
    for type_url in [None, Some("type.googleapis.com/test.Response")] {
        check_reply(type_url).await;
    }
}

async fn check_reply(type_url: Option<&'static str>) {
    // Arrange
    let (client_id_tx, client_ids) = flume::bounded(2);
    let handler =
        move |_: &mut NoState, conn_req, clients: &ConnectedClients| match conn_req {
            Msg::ConnectRequest(ConnectRequest { client_id, .. }) => ConnectResponse {
                client_id: client_id.unwrap().id.clone(),
                success: true,
                error: "nothing".to_string(),
            }
            .into_res(),

            Msg::Payload(payload) => {
                let expected_type = if payload.src.as_ref().unwrap().id == "boo" {
                    type_url.unwrap_or_default()
                } else {
                    ""
                };
                assert_eq!(payload.payload.as_ref().unwrap().type_url, expected_type);
                clients.send(payload);
                None
            }

            _ => None,
        };
    let sv =
        TestServer::with_interceptor(NoState, handler, move |request: Request<()>| {
            let client_id = request
                .metadata()
                .get("x-client-id")
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    Status::invalid_argument("Missing or invalid x-client-id")
                })?;
            client_id_tx.try_send(client_id.to_owned()).unwrap();
            Ok(request)
        })
        .await;

    let opts_foo = ClientOpts::entity(EntityType::App)
        .src_id("foo")
        .namespace("bar")
        .endpoint(format!("https://{}", sv.addr()))
        .auth(Auth::Token(Default::default()))
        .max_connection_attempts(Amount::Val(1))
        .connection_timeout(Duration::from_secs(5))
        .heartbeat(Duration::from_secs(u64::MAX))
        .ack_timeout(Duration::from_millis(1)) // so we can test that we aren't retrying when QoS is AtMostOnce
        .build();

    let opts_boo = ClientOpts::entity(EntityType::App)
        .src_id("boo")
        .client_id("boo-client")
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

    // Act
    let msg = SendMessage::to(EntityType::App)
        .id("boo")
        .namespace("bar")
        .payload(b"woah");

    let reply_task = task::spawn(async move {
        let msg = client_boo.recv().await.unwrap();
        let builder = msg.reply(b"whats up??");
        if let Some(type_url) = type_url {
            builder.type_url(type_url).send().await.unwrap();
        } else {
            builder.send().await.unwrap();
        }
    });

    let res = timeout(Duration::from_secs(10), client_foo.ask(msg))
        .await
        .unwrap()
        .unwrap();
    reply_task.await.unwrap();

    // Assert
    assert_eq!(res, b"whats up??");

    let mut client_ids: Vec<_> = client_ids.drain().collect();
    client_ids.sort_unstable();
    assert_eq!(client_ids, ["boo-client", "foo"]);
}
