/*
* tests RelayClient attempts to connect over and over again until it reaches maximum determined number of attempts
*/
mod test_server;

use orb_relay_messages::relay::{
    entity::EntityType, relay_connect_request::Msg, ConnectRequest, ConnectResponse,
};
use orb_relay_client::{Amount, Client, ClientOpts};
use std::time::Duration;
use test_server::{IntoRes, TestServer};
use tokio::time;

#[tokio::test]
async fn connects() {
    // Arrange
    let sv = TestServer::new((false, 0), |state, conn_req| {
        if let Msg::ConnectRequest(ConnectRequest { client_id, .. }) = conn_req {
            state.0 = true;
            state.1 += 1;

            ConnectResponse {
                client_id: client_id.unwrap().id.clone(),
                success: true,
                error: "nothing".to_string(),
            }
            .into_res()
        } else {
            panic!("wrong msg")
        }
    })
    .await;

    let opts = ClientOpts::entity(EntityType::App)
        .id("foo")
        .namespace("bar")
        .endpoint(format!("http://{}", sv.addr()))
        .auth_token(String::default())
        .max_connection_attempts(Amount::Val(1))
        .connection_timeout(Duration::from_millis(10))
        .build();

    // Act
    let (_client, _handle) = Client::connect(opts);

    // Assert
    time::sleep(Duration::from_millis(50)).await;
    let (is_connected, attempts) = *sv.state().await;
    assert!(is_connected);
    assert_eq!(attempts, 1);
}

#[tokio::test]
async fn tries_to_connect_the_expected_number_of_times_then_gives_up() {
    // Arrange
    let expected_attempts = 2;
    let sv = TestServer::new(0, |attempts, _conn_req| {
        *attempts += 1;
        ConnectResponse {
            client_id: "doesntmatter".to_string(),
            success: false,
            error: "nothing".to_string(),
        }
        .into_res()
    })
    .await;

    let opts = ClientOpts::entity(EntityType::App)
        .id("foo")
        .namespace("bar")
        .endpoint(format!("http://{}", sv.addr()))
        .auth_token(String::default())
        .max_connection_attempts(Amount::Val(expected_attempts))
        .connection_timeout(Duration::from_millis(10))
        .build();

    // Act
    let (_client, handle) = Client::connect(opts);
    let res = handle.await.unwrap();

    // Assert
    assert!(res.is_err());
    let actual_attempts = sv.state().await;
    assert_eq!(*actual_attempts, expected_attempts);
}
