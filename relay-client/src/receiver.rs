use crate::{flume_receiver_stream, ClientId, RecvMessage, SendMessage, Seq};
use color_eyre::eyre::{self, eyre, Context};
use derive_more::From;
use orb_relay_messages::relay::{
    connect_request::AuthMethod, entity::EntityType, relay_connect_request,
    relay_connect_response, relay_service_client::RelayServiceClient, Ack,
    ConnectRequest, ConnectResponse, Entity, RelayConnectRequest, RelayConnectResponse,
    RelayPayload,
};
use std::{collections::HashMap, time::Duration};
use tokio::{task, time};
use tokio_stream::StreamExt;
use tonic::{
    transport::{self, ClientTlsConfig, Endpoint},
    Streaming,
};
use tracing::{debug, error};

#[derive(Default)]
pub struct State {
    pending_acks: HashMap<Seq, (SendMessage, flume::Sender<()>)>,
    pending_replies: HashMap<(ClientId, Seq), flume::Sender<RecvMessage>>,
}

pub struct Props {
    domain: String,
    auth_token: String,
    client_id: String,
    namespace: String,
    entity_type: EntityType,
    client_tx: flume::Sender<RecvMessage>,
    tonic_tx: flume::Sender<RelayConnectRequest>,
    tonic_rx: flume::Receiver<RelayConnectRequest>,
    connection_retry: Duration,
}

pub enum Msg {
    WaitForAck {
        original_msg: SendMessage,
        ack_tx: flume::Sender<()>,
        retry_strategy: RetryStrategy,
    },

    RetryAck {
        seq: Seq,
        retry_strategy: RetryStrategy,
    },

    WaitForReply {
        from_client: ClientId,
        seq: Seq,
        recv_msg_tx: flume::Sender<RecvMessage>,
    },

    RetryAsk {
        from_client: ClientId,
        seq: Seq,
    },

    Stop,
}

#[derive(From, Debug)]
pub enum Err {
    StopRequest,
    StreamEnded,
    Transport(transport::Error),
    Tonic(tonic::Status),
    Other(eyre::Error),
}

pub struct RetryStrategy {
    retries_left: u64,
    backoff: Duration,
}

pub fn run(props: Props) -> flume::Sender<Msg> {
    let (relay_actor_tx, relay_actor_rx) = flume::unbounded();

    task::spawn(async move {
        let mut state = State::default();

        loop {
            match main_loop(&mut state, &props, relay_actor_rx.clone()).await {
                Err(Err::StopRequest) => {
                    break;
                }

                Err(e) => {
                    error!(
                        "RelayClient errored out {e:?}. Retrying in {}s",
                        props.connection_retry.as_secs()
                    );
                }

                Ok(()) => (),
            }
        }
    });

    relay_actor_tx
}

async fn main_loop(
    state: &mut State,
    props: &Props,
    relay_actor_rx: flume::Receiver<Msg>,
) -> Result<(), Err> {
    let mut response_stream = time::timeout(Duration::from_secs(5), connect(props))
        .await
        .wrap_err("Timed out trying to establish a connection")??;

    loop {
        tokio::select! {
            biased;

            msg = relay_actor_rx.recv_async() => {
                let msg = msg.wrap_err("relay_actor_tx seemingly dropped")?;
                handle_msg(state, props, msg)?;

            }

            Some(message) = response_stream.next() => {
                match message?.msg {
                    Some(relay_connect_response::Msg::Payload(RelayPayload {
                        payload: Some(payload),
                        src: Some(entity),
                        seq,
                        ..
                    })) => {
                        let recv_msg = RecvMessage::new(entity, payload.value);
                        handle_payload(state, recv_msg, seq, &props.client_tx)?;
                    }

                    Some(relay_connect_response::Msg::Ack(Ack { seq })) => handle_ack(state, seq),

                    Some(other_msg) => {
                        debug!(" Received a non-Any message: {:?}", other_msg)
                    }

                    None => (),
                }
            }
        }
    }
}

fn handle_msg(state: &mut State, props: &Props, msg: Msg) -> Result<(), Err> {
    match msg {
        Msg::Stop => return Err(Err::StopRequest),

        Msg::WaitForAck {
            original_msg,
            ack_tx,
            retry_strategy,
        } => {}

        Msg::WaitForReply { .. } => (),

        Msg::RetryAck {
            seq,
            retry_strategy,
        } => todo!(),

        Msg::RetryAsk { from_client, seq } => todo!(),
    }

    Ok(())
}

fn handle_payload(
    state: &mut State,
    recv_msg: RecvMessage,
    seq: Seq,
    client_tx: &flume::Sender<RecvMessage>,
) -> Result<(), Err> {
    let key = (recv_msg.from.id.clone(), seq);
    if let Some(reply) = state.pending_replies.remove(&key) {
        reply.send(recv_msg);
    } else {
        client_tx
            .send(recv_msg)
            .wrap_err("Failed to send message to client_rx")?;
    }

    Ok(())
}

fn handle_ack(state: &mut State, seq: Seq) {
    if let Some((_pending_msg, ack_tx)) = state.pending_acks.remove(&seq) {
        ack_tx.send(());
    };
}

async fn connect(props: &Props) -> Result<Streaming<RelayConnectResponse>, Err> {
    let Props {
        domain,
        auth_token,
        client_id,
        namespace,
        entity_type,
        tonic_tx,
        tonic_rx,
        ..
    } = props;

    let tls_config = ClientTlsConfig::new().with_native_roots();

    let channel = Endpoint::from_shared(format!("https://{}", domain))?
        .tls_config(tls_config)?
        .keep_alive_while_idle(true)
        .http2_keep_alive_interval(Duration::from_secs(5))
        .keep_alive_timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(20))
        .connect()
        .await?;

    let mut relay_client = RelayServiceClient::new(channel);

    tonic_tx
        .send(RelayConnectRequest {
            msg: Some(relay_connect_request::Msg::ConnectRequest(ConnectRequest {
                client_id: Some(Entity {
                    id: client_id.clone(),
                    entity_type: *entity_type as i32,
                    namespace: namespace.clone(),
                }),
                auth_method: Some(AuthMethod::Token(auth_token.clone())),
            })),
        })
        .wrap_err("Failed to send RelayConnectRequest")?;

    let mut response_stream: Streaming<RelayConnectResponse> = relay_client
        .relay_connect(flume_receiver_stream::new(tonic_rx.clone(), 4))
        .await?
        .into_inner();

    while let Some(message) = response_stream.next().await {
        match message?.msg {
            Some(relay_connect_response::Msg::ConnectResponse(ConnectResponse {
                success: true,
                ..
            })) => {
                debug!("Connection established successfully.");
                break;
            }

            Some(relay_connect_response::Msg::ConnectResponse(ConnectResponse {
                success: false,
                ..
            })) => {
                debug!("Failed to establish connection.");
                return Err(eyre!("Failed to establish connection.").into());
            }

            Some(other_msg) => {
                debug!(" Received unexpected message: {:?}", other_msg);
            }

            None => (),
        }
    }

    Ok(response_stream)
}
