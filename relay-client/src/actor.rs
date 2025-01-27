use crate::{
    flume_receiver_stream, relay_payload, Amount, Auth, ClientId, ClientOpts, Err, QoS,
    RecvdRelayPayload, SendMessage, Seq,
};
use color_eyre::eyre::{eyre, Context};
use orb_relay_messages::relay::{
    connect_request::AuthMethod, relay_connect_request, relay_connect_response,
    relay_service_client::RelayServiceClient, Ack, ConnectRequest, ConnectResponse,
    Entity, RelayConnectRequest, RelayConnectResponse, RelayPayload,
};
use secrecy::ExposeSecret;
use std::collections::HashMap;
use tokio::{task, time};
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;
use tonic::{
    transport::{ClientTlsConfig, Endpoint},
    Streaming,
};
use tracing::{debug, error, info};

#[derive(Default)]
pub struct State {
    pending_acks: HashMap<Seq, (SendMessage, flume::Sender<()>)>,
    pending_replies:
        HashMap<(ClientId, Seq), (SendMessage, flume::Sender<RecvdRelayPayload>)>,
    heartbeat_started: bool,
}

pub struct Props {
    pub opts: ClientOpts,
    pub client_tx: flume::Sender<RecvdRelayPayload>,
    pub tonic_tx: flume::Sender<RelayConnectRequest>,
    pub tonic_rx: flume::Receiver<RelayConnectRequest>,
}

#[derive(Debug)]
pub(crate) enum Msg {
    RelayResponse(relay_connect_response::Msg),

    Send(RelayConnectRequest),

    WaitForAck {
        original_msg: SendMessage,
        ack_tx: flume::Sender<()>,
        seq: Seq,
    },

    WaitForReply {
        original_msg: SendMessage,
        recv_msg_tx: flume::Sender<RecvdRelayPayload>,
        seq: Seq,
    },

    RetryAck {
        seq: Seq,
        retries_left: Amount,
    },

    RetryAsk {
        from_client: ClientId,
        seq: Seq,
        retries_left: Amount,
    },

    Stop,

    Heartbeat,
}

pub fn run(props: Props) -> (flume::Sender<Msg>, task::JoinHandle<Result<(), Err>>) {
    let (relay_actor_tx, relay_actor_rx) = flume::unbounded();

    let relay_actor_tx_clone = relay_actor_tx.clone();
    let join_handle = task::spawn(async move {
        let mut state = State::default();
        let mut conn_attempts = 0_u64;

        loop {
            conn_attempts += 1;

            let cancellation_token = CancellationToken::new();
            let result = main_loop(
                &mut state,
                &props,
                relay_actor_tx_clone.clone(),
                relay_actor_rx.clone(),
                cancellation_token.clone(),
            )
            .await;

            if let Err(e) = result {
                cancellation_token.cancel();
                if let Err::StopRequest = e {
                    return Err(Err::StopRequest);
                } else if props.opts.max_connection_attempts <= conn_attempts {
                    return Err(e);
                } else {
                    error!(
                        "RelayClient errored out {e:?}. Retrying in {}s",
                        props.opts.connection_timeout.as_secs()
                    );

                    time::sleep(props.opts.connection_backoff).await;
                }
            };
        }
    });

    (relay_actor_tx, join_handle)
}

async fn main_loop(
    state: &mut State,
    props: &Props,
    relay_actor_tx: flume::Sender<Msg>,
    relay_actor_rx: flume::Receiver<Msg>,
    cancellation_token: CancellationToken,
) -> Result<(), Err> {
    let mut response_stream = time::timeout(
        props.opts.connection_timeout,
        connect(props, &relay_actor_tx, cancellation_token),
    )
    .await
    .wrap_err("Timed out trying to establish a connection")??;

    if !state.heartbeat_started {
        state.heartbeat_started = true;
        let heartbeat = props.opts.heartbeat;
        let relay_actor_tx = relay_actor_tx.clone();
        task::spawn(async move {
            time::sleep(heartbeat).await;
            let _ = relay_actor_tx.send(Msg::Heartbeat);
        });
    }

    loop {
        let msg = tokio::select! {
            biased;

            msg = relay_actor_rx.recv_async() => {
                msg.map(Some)
                   .wrap_err("relay_actor_tx seemingly dropped")?
            }

            message = response_stream.next() => {
                message.ok_or(Err::StreamEnded)?
                    .map(|msg|msg.msg)?
                    .map(Msg::RelayResponse)
            }
        };

        if let Some(msg) = msg {
            handle_msg(state, props, msg, &relay_actor_tx)?;
        }
    }
}

/// This loop only runs if we have a connection.
fn handle_msg(
    state: &mut State,
    props: &Props,
    msg: Msg,
    relay_actor_tx: &flume::Sender<Msg>,
) -> Result<(), Err> {
    match msg {
        Msg::RelayResponse(relay_connect_response::Msg::Payload(RelayPayload {
            payload: Some(payload),
            src: Some(entity),
            seq,
            ..
        })) => {
            let recv_msg = RecvdRelayPayload {
                from: entity,
                payload: payload.value,
                seq,
            };
            handle_payload(state, recv_msg, seq, &props.client_tx)?;
        }

        Msg::RelayResponse(relay_connect_response::Msg::Ack(Ack { seq })) => {
            handle_ack(state, seq)
        }

        Msg::RelayResponse(other_msg) => {
            debug!(" Received a non-Any message: {:?}", other_msg)
        }

        Msg::Send(payload) => {
            props
                .tonic_tx
                .send(payload)
                .wrap_err("Failed to send a payload to tonic_tx")?;
        }

        Msg::Stop => return Err(Err::StopRequest),

        Msg::WaitForAck {
            original_msg,
            ack_tx,
            seq,
        } => {
            state.pending_acks.insert(seq, (original_msg, ack_tx));

            let backoff = props.opts.ack_timeout;
            let retries_left = props.opts.max_message_attempts;
            let tx = relay_actor_tx.clone();

            task::spawn(async move {
                time::sleep(backoff).await;
                let _ = tx.send(Msg::RetryAck { seq, retries_left });
            });
        }

        Msg::WaitForReply {
            original_msg,
            seq,
            recv_msg_tx,
        } => {
            state.pending_replies.insert(
                (original_msg.target_id.clone(), seq),
                (original_msg.clone(), recv_msg_tx.clone()),
            );

            let backoff = props.opts.reply_timeout;
            let retries_left = match original_msg.qos {
                QoS::AtMostOnce => Amount::Val(0),
                QoS::AtLeastOnce => props.opts.max_message_attempts,
            };

            let tx = relay_actor_tx.clone();

            task::spawn(async move {
                time::sleep(backoff).await;
                let _ = tx.send(Msg::RetryAsk {
                    from_client: original_msg.target_id,
                    seq,
                    retries_left,
                });
            });
        }

        Msg::RetryAck { seq, retries_left } => {
            if let Some((msg, _sender)) = state.pending_acks.get(&seq) {
                if retries_left.is_zero() {
                    error!(
                        "Failed to send message with seq {seq} after multiple retries."
                    );

                    return Ok(());
                }

                let payload = relay_payload(&props.opts, msg, seq);
                props
                    .tonic_tx
                    .send(payload.into())
                    .wrap_err("Failed to send retry message to tonic_tx")?;

                let backoff = props.opts.ack_timeout;
                let retries_left = props.opts.max_message_attempts;
                let tx = relay_actor_tx.clone();

                task::spawn(async move {
                    time::sleep(backoff).await;
                    let _ = tx.send(Msg::RetryAck {
                        seq,
                        retries_left: retries_left.decrement(),
                    });
                });
            }
        }

        Msg::RetryAsk {
            from_client,
            seq,
            retries_left,
        } => {
            if let Some((msg, _sender)) =
                state.pending_replies.get(&(from_client.clone(), seq))
            {
                if retries_left.is_zero() {
                    error!(
                        "Failed to send message with seq {seq} after multiple retries."
                    );

                    return Ok(());
                }

                let payload = relay_payload(&props.opts, msg, seq);
                props
                    .tonic_tx
                    .send(payload.into())
                    .wrap_err("Failed to send retry message to tonic_tx")?;

                let backoff = props.opts.reply_timeout;
                let retries_left = props.opts.max_message_attempts;
                let tx = relay_actor_tx.clone();

                task::spawn(async move {
                    time::sleep(backoff).await;
                    let _ = tx.send(Msg::RetryAsk {
                        from_client,
                        seq,
                        retries_left: retries_left.decrement(),
                    });
                });
            }
        }

        Msg::Heartbeat => {
            props
                .tonic_tx
                .send(
                    orb_relay_messages::relay::Heartbeat {
                        // we don't care about seq here
                        // this is just to maintain connection alive
                        seq: 0,
                    }
                    .into(),
                )
                .wrap_err("Failed to send heartbeat to Orb Relay Server")?;

            let relay_actor_tx = relay_actor_tx.clone();
            let heartbeat = props.opts.heartbeat;
            task::spawn(async move {
                time::sleep(heartbeat).await;
                if relay_actor_tx.send(Msg::Heartbeat).is_err() {
                    error!("Failed to send heartbeat as relay_actor_rx seems to have been dropped.");
                }
            });
        }
    }

    Ok(())
}

fn handle_payload(
    state: &mut State,
    recv_msg: RecvdRelayPayload,
    seq: Seq,
    client_tx: &flume::Sender<RecvdRelayPayload>,
) -> color_eyre::Result<()> {
    let key = (recv_msg.from.id.clone(), seq);
    if let Some((_msg, reply)) = state.pending_replies.remove(&key) {
        if reply.send(recv_msg).is_err() {
            error!("Failed to send reply from message with seq {seq}. Seems like scope containing .ask call was dropped.");
        }
    } else {
        client_tx
            .send(recv_msg)
            .wrap_err("Failed to send message to client_rx")?;
    }

    Ok(())
}

fn handle_ack(state: &mut State, seq: Seq) {
    if let Some((_pending_msg, ack_tx)) = state.pending_acks.remove(&seq) {
        let _ = ack_tx.send(());
    };
}

async fn connect(
    props: &Props,
    relay_actor_tx: &flume::Sender<Msg>,
    cancellation_token: CancellationToken,
) -> Result<Streaming<RelayConnectResponse>, Err> {
    let Props {
        opts,
        tonic_tx,
        tonic_rx,
        ..
    } = props;

    let ClientOpts {
        endpoint: domain,
        auth,
        client_id,
        namespace,
        entity_type,
        ..
    } = opts;

    let mut endpoint =
        Endpoint::from_shared(domain.clone())?.keep_alive_while_idle(true);

    if domain.starts_with("https://") {
        let tls_config = ClientTlsConfig::new().with_native_roots();
        endpoint = endpoint.tls_config(tls_config)?;
    }

    let channel = endpoint.connect().await?;

    let mut relay_client = RelayServiceClient::new(channel);

    tonic_tx
        .send(RelayConnectRequest {
            msg: Some(relay_connect_request::Msg::ConnectRequest(ConnectRequest {
                client_id: Some(Entity {
                    id: client_id.clone(),
                    entity_type: *entity_type as i32,
                    namespace: namespace.clone(),
                }),
                auth_method: Some(match auth {
                    Auth::Token(t) => AuthMethod::Token(t.expose_secret().to_string()),
                    Auth::Zkp => todo!(),
                }),
            })),
        })
        .wrap_err("Failed to send RelayConnectRequest")?;

    let mut response_stream: Streaming<RelayConnectResponse> = relay_client
        .relay_connect(flume_receiver_stream::new(
            tonic_rx.clone(),
            4,
            cancellation_token,
        ))
        .await?
        .into_inner();

    while let Some(message) = response_stream.next().await {
        match message?.msg {
            Some(relay_connect_response::Msg::ConnectResponse(ConnectResponse {
                success: true,
                ..
            })) => {
                info!("Connection established successfully.");
                break;
            }

            Some(relay_connect_response::Msg::ConnectResponse(ConnectResponse {
                success: false,
                ..
            })) => {
                return Err(eyre!("Failed to establish connection.").into());
            }

            Some(msg) => {
                relay_actor_tx
                    .send(Msg::RelayResponse(msg))
                    .wrap_err("relay_actor_tx receivers unexpectedly dropped")?;
            }

            None => (),
        }
    }

    Ok(response_stream)
}
