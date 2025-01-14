use crate::{
    flume_receiver_stream, Amount, ClientId, ClientOpts, QoS, RecvMessage,
    RetryStrategy, SendMessage, Seq,
};
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
    pub opts: ClientOpts,
    pub client_tx: flume::Sender<RecvMessage>,
    pub tonic_tx: flume::Sender<RelayConnectRequest>,
    pub tonic_rx: flume::Receiver<RelayConnectRequest>,
}

pub enum Msg {
    WaitForAck {
        original_msg: SendMessage,
        ack_tx: flume::Sender<()>,
        seq: Seq,
    },

    WaitForReply {
        from_client: ClientId,
        seq: Seq,
        recv_msg_tx: flume::Sender<RecvMessage>,
        qos: QoS,
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
}

#[derive(From, Debug)]
pub enum Err {
    StopRequest,
    StreamEnded,
    Transport(transport::Error),
    Tonic(tonic::Status),
    Other(eyre::Error),
}

pub fn run(props: Props) -> (flume::Sender<Msg>, task::JoinHandle<()>) {
    let (relay_actor_tx, relay_actor_rx) = flume::unbounded();

    let relay_actor_tx_clone = relay_actor_tx.clone();
    let join_handle = task::spawn(async move {
        let mut state = State::default();

        loop {
            match main_loop(
                &mut state,
                &props,
                relay_actor_tx_clone.clone(),
                relay_actor_rx.clone(),
            )
            .await
            {
                Err(Err::StopRequest) => {
                    break;
                }

                Err(e) => {
                    error!(
                        "RelayClient errored out {e:?}. Retrying in {}s",
                        props.opts.connection_timeout.as_secs()
                    );
                }

                Ok(()) => (),
            }
        }
    });

    (relay_actor_tx, join_handle)
}

async fn main_loop(
    state: &mut State,
    props: &Props,
    relay_actor_tx: flume::Sender<Msg>,
    relay_actor_rx: flume::Receiver<Msg>,
) -> Result<(), Err> {
    let mut response_stream =
        time::timeout(props.opts.connection_timeout, connect(props))
            .await
            .wrap_err("Timed out trying to establish a connection")??;

    loop {
        tokio::select! {
            biased;

            msg = relay_actor_rx.recv_async() => {
                let msg = msg.wrap_err("relay_actor_tx seemingly dropped")?;
                handle_msg(state, props, msg, &relay_actor_tx)?;

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

fn handle_msg(
    state: &mut State,
    props: &Props,
    msg: Msg,
    relay_actor_tx: &flume::Sender<Msg>,
) -> Result<(), Err> {
    match msg {
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
            from_client,
            seq,
            recv_msg_tx,
            qos,
        } => {
            state
                .pending_replies
                .insert((from_client.clone(), seq), recv_msg_tx.clone());

            let backoff = props.opts.reply_timeout;
            let retries_left = match qos {
                QoS::AtMostOnce => Amount::Val(0),
                QoS::AtLeastOnce => props.opts.max_message_attempts,
            };

            let tx = relay_actor_tx.clone();

            task::spawn(async move {
                time::sleep(backoff).await;
                let _ = tx.send(Msg::RetryAsk {
                    from_client,
                    seq,
                    retries_left,
                });
            });
        }

        Msg::RetryAck { seq, retries_left } => {
            if let Some((_msg, _sender)) = state.pending_acks.get(&seq) {
                if retries_left.is_nonzero() {
                    // TODO: resend message here

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
                } else {
                    error!(
                        "Failed to send message with seq {seq} after multiple retries."
                    );
                }
            }
        }

        Msg::RetryAsk {
            from_client,
            seq,
            retries_left,
        } => {
            if retries_left.is_nonzero() {
                // TODO: resend message here

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
            } else {
                error!("Failed to send message with seq {seq} after multiple retries.");
            }
        }
    }

    Ok(())
}

fn handle_payload(
    state: &mut State,
    recv_msg: RecvMessage,
    seq: Seq,
    client_tx: &flume::Sender<RecvMessage>,
) -> color_eyre::Result<()> {
    let key = (recv_msg.from.id.clone(), seq);
    if let Some(reply) = state.pending_replies.remove(&key) {
        let _ = reply.send(recv_msg); // TODO: should we care about this error?
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

async fn connect(props: &Props) -> Result<Streaming<RelayConnectResponse>, Err> {
    let Props {
        opts,
        tonic_tx,
        tonic_rx,
        ..
    } = props;

    let ClientOpts {
        domain,
        auth_token,
        client_id,
        namespace,
        entity_type,
        ..
    } = opts;

    let tls_config = ClientTlsConfig::new().with_native_roots();

    let channel = Endpoint::from_shared(format!("https://{}", domain))?
        .tls_config(tls_config)?
        .keep_alive_while_idle(true)
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
