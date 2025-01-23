/*
* tests RelayClient attempts to connect over and over again until it reaches maximum determined number of attempts
*/

use std::{sync::Arc, time::Duration};

use orb_relay_messages::relay::{entity::EntityType, relay_connect_request::Msg, ConnectRequest, ConnectResponse};

mod test_server;
use relay_client::{Amount, Client, ClientOpts};
use test_server::TestServer;
use tokio::{sync::Mutex, time};

#[tokio::test]
async fn connects() {
    // Arrange
    let sv = TestServer::new(move |mut stream| {
        async_stream::stream! {
            let msg = stream.message().await.unwrap().unwrap().msg.unwrap();
            if let Msg::ConnectRequest(ConnectRequest { client_id, .. }) = msg {
                yield ConnectResponse { client_id: client_id.unwrap().id.clone(), success: true, error: "nothing".to_string() }
            }
        }
    }).await;

    let opts = ClientOpts::entity(EntityType::App)
        .id("foo")
        .namespace("bar")
        .endpoint(format!("http://{}", sv.addr.to_string()))
        .auth_token(String::default())
        .max_connection_attempts(Amount::Val(1))
        .connection_timeout(Duration::from_millis(10))
        .build();

    // Act
    let (_client, handle) = Client::connect(opts);

    // Assert
    time::sleep(Duration::from_millis(50)).await;
    assert!(!handle.is_finished());
}

#[tokio::test]
async fn tries_to_connect_the_expected_number_of_times_then_gives_up() {
    // Arrange
    let expected_attempts = 2;
    let actual_attempts = Arc::new(Mutex::new(0));

    let attempts_clone = actual_attempts.clone();
    let sv = TestServer::new(move |_stream| {
        let attempts = attempts_clone.clone();

        async_stream::stream! {
            let mut state = attempts.lock().await;
            *state += 1;
            yield ConnectResponse { client_id: "doesntmatter".to_string(), success: false, error: "nothing".to_string() }
        }
    })
    .await;

    let opts = ClientOpts::entity(EntityType::App)
        .id("foo")
        .namespace("bar")
        .endpoint(format!("http://{}", sv.addr.to_string()))
        .auth_token(String::default())
        .max_connection_attempts(Amount::Val(expected_attempts))
        .connection_timeout(Duration::from_millis(10))
        .build();

    // Act
    let (_client, handle) = Client::connect(opts);
    let res = handle.await.unwrap();

    // Assert
    let count = actual_attempts.lock().await;
    assert!(res.is_err());
    assert_eq!(*count, expected_attempts);
}
