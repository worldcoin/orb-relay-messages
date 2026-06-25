#![cfg(not(feature = "dangerously-allow-http"))]

use orb_relay_client::{Amount, Auth, Client, ClientOpts};
use orb_relay_messages::relay::entity::EntityType;
use secrecy::SecretString;

#[tokio::test]
async fn plain_http_is_rejected_without_allow_http() {
    let opts = ClientOpts::entity(EntityType::App)
        .id("client")
        .namespace("default")
        .endpoint("http://127.0.0.1:1")
        .auth(Auth::Token(SecretString::new("token".into())))
        .connection_timeout(std::time::Duration::from_millis(50))
        .max_connection_attempts(Amount::Val(1))
        .build();

    let (_client, handle) = Client::connect(opts);
    let err = handle.await.unwrap().unwrap_err();

    assert!(format!("{err:?}").contains("HTTP relay endpoints are disabled"));
}
