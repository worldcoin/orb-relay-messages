mod test_server;

use std::time::Duration;

use orb_relay_messages::relay::{
    entity::EntityType, relay_connect_request::Msg, ConnectRequest, ConnectResponse,
};
use orb_relay_client::{Amount, Client, ClientOpts};
use test_server::{IntoRes, TestServer};
use tokio::time;

#[tokio::test]
async fn it_sends_heartbeat_periodically() {
    // Arrange
    let sv = TestServer::new(0, |heartbeats, conn_req| match conn_req {
        Msg::ConnectRequest(ConnectRequest { client_id, .. }) => ConnectResponse {
            client_id: client_id.unwrap().id.clone(),
            success: true,
            error: "nothing".to_string(),
        }
        .into_res(),

        Msg::Heartbeat(_) => {
            *heartbeats += 1;
            None
        }

        msg => panic!("unexpected msg {msg:?}"),
    })
    .await;

    let opts = ClientOpts::entity(EntityType::App)
        .id("foo")
        .namespace("bar")
        .endpoint(format!("http://{}", sv.addr()))
        .auth_token(String::default())
        .max_connection_attempts(Amount::Val(1))
        .connection_timeout(Duration::from_millis(10))
        .heartbeat(Duration::from_millis(50))
        .build();

    // Act
    let (_client, _handle) = Client::connect(opts);

    // Assert
    time::sleep(Duration::from_millis(200)).await;
    let heartbeat_count = *sv.state().await;
    assert_eq!(heartbeat_count, 3)
}
