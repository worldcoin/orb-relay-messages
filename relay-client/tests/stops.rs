mod test_server;

use std::time::Duration;

use orb_relay_messages::relay::{
    entity::EntityType, relay_connect_request::Msg, ConnectRequest, ConnectResponse,
};
use orb_relay_client::{Amount, Client, ClientOpts, Err, SendMessage};
use test_server::{IntoRes, TestServer};
use tokio::time;

#[tokio::test]
async fn it_stops_sever_when_stop_is_called() {
    // Arrange
    let sv = TestServer::new(0, |msg_count, conn_req| {
        *msg_count += 1;

        match conn_req {
            Msg::ConnectRequest(ConnectRequest { client_id, .. }) => ConnectResponse {
                client_id: client_id.unwrap().id.clone(),
                success: true,
                error: "nothing".to_string(),
            }
            .into_res(),

            Msg::Heartbeat(_) => None,

            msg => panic!("got unexpected msg {msg:?}"),
        }
    })
    .await;

    let opts = ClientOpts::entity(EntityType::App)
        .id("foo")
        .namespace("bar")
        .endpoint(format!("http://{}", sv.addr().to_string()))
        .auth_token(String::default())
        .max_connection_attempts(Amount::Val(1))
        .connection_timeout(Duration::from_millis(10))
        .build();

    // Act
    let (client, handle) = Client::connect(opts);
    time::sleep(Duration::from_millis(50)).await;
    client.stop().await.unwrap();
    // message below should be ignored as we already requested client to stop
    client
        .send(
            SendMessage::to(EntityType::App)
                .id("foo")
                .namespace("bar")
                .payload(b"foobar"),
        )
        .await
        .unwrap();

    // Assert
    assert_eq!(*sv.state().await, 1);
    let stop_fut = async {
        match handle.await {
            Ok(Err(Err::StopRequest)) => (),
            other => panic!("unexpected terminatin of client handle: {other:?}"),
        }
    };

    time::timeout(Duration::from_millis(100), stop_fut)
        .await
        .unwrap();
}
