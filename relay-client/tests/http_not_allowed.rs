use orb_relay_client::{Auth, Client, ClientOpts};
use orb_relay_messages::relay::entity::EntityType;
use secrecy::SecretString;

#[tokio::test]
async fn plain_http_is_rejected() {
    let opts = ClientOpts::entity(EntityType::App)
        .id("client")
        .namespace("default")
        .endpoint("http://127.0.0.1:1")
        .auth(Auth::Token(SecretString::new("token".into())))
        .build();

    let (_client, handle) = Client::connect(opts);
    let err = handle.await.unwrap().unwrap_err();

    assert!(format!("{err:?}").contains("must use https"));
}
