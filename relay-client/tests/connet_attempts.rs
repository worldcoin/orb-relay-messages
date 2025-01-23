/*
* tests RelayClient attempts to connect over and over again until it reaches maximum determined number of attempts
*/

use std::{sync::Arc, time::Duration};

use orb_relay_messages::relay::{entity::EntityType, Ack};

mod test_server;
use relay_client::{Amount, Client, ClientOpts};
use test_server::TestServer;
use tokio::{sync::Mutex, time};

#[tokio::test]
async fn tries_to_connect_the_expected_number_of_times_then_gives_up() {
    // Arrange
    let x = Arc::new(Mutex::new(0));
    println!("about to start the server lets goooo");
    let sv = TestServer::new(move || {
        let state = x.clone();
        |msg| async {
            let mut s = state.lock().await;
            *s += 1;
            println!("got msg: {msg:?}!!");
            Ack { seq: 0 }
        }
    })
    .await;

    let opts = ClientOpts::entity(EntityType::App)
        .id("foo")
        .namespace("bar")
        .domain(sv.addr.to_string())
        .auth_token(String::default())
        .max_connection_attempts(Amount::Val(1))
        .build();

    let (client, handle) = Client::connect(opts);

    // Act
    println!("actually connected {:?}", sv.addr);
    handle.await.unwrap().unwrap();
    // Assert
}
