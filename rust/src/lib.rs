pub use ::prost;
pub use ::prost_types;
pub use ::tonic;

use prost::{DecodeError, EncodeError, Message, Name};

uniffi::setup_scaffolding!();

pub mod relay {
    tonic::include_proto!("relay");

    impl From<RelayPayload> for RelayConnectRequest {
        fn from(payload: RelayPayload) -> Self {
            RelayConnectRequest {
                req_msg: Some(relay_connect_request::ReqMsg::Payload(payload)),
            }
        }
    }

    impl From<Heartbeat> for RelayConnectRequest {
        fn from(heartbeat: Heartbeat) -> Self {
            RelayConnectRequest {
                req_msg: Some(relay_connect_request::ReqMsg::Heartbeat(heartbeat)),
            }
        }
    }

    impl From<ConnectRequest> for RelayConnectRequest {
        fn from(connect_request: ConnectRequest) -> Self {
            RelayConnectRequest {
                req_msg: Some(relay_connect_request::ReqMsg::ConnectRequest(
                    connect_request,
                )),
            }
        }
    }
}

pub mod common {
    pub mod v1 {
        tonic::include_proto!("common.v1");
    }
}

pub mod self_serve {
    pub mod app {
        pub mod v1 {
            tonic::include_proto!("self_serve.app.v1");
        }
    }
    pub mod orb {
        pub mod v1 {
            tonic::include_proto!("self_serve.orb.v1");
        }
    }
}

pub mod config {
    tonic::include_proto!("config");
}

// Mozilla's UniFFI is not able to provide bindings for types that are not directly annotated with #[derive(Record)] (or
// some other). For this we are in the difficult position of having to copy the Any and Timestamp types directly from
// the source code of the protobuf crate. Here are the relevant parts of the source code:
// - https://github.com/tokio-rs/prost/blob/7968f906d2d1cf8193183873fecb025d18437cd8/prost-types/src/any.rs#L3
// - https://github.com/tokio-rs/prost/blob/7968f906d2d1cf8193183873fecb025d18437cd8/prost-types/src/type_url.rs#L17
// - https://github.com/tokio-rs/prost/blob/7968f906d2d1cf8193183873fecb025d18437cd8/prost-types/src/protobuf.rs#L1242

#[derive(Clone, Copy, PartialEq, ::prost::Message, ::uniffi::Record)]
pub struct Timestamp {
    #[prost(int64, tag = "1")]
    pub seconds: i64,
    #[prost(int32, tag = "2")]
    pub nanos: i32,
}

#[derive(Clone, PartialEq, ::prost::Message, ::uniffi::Record)]
pub struct Any {
    #[prost(string, tag = "1")]
    pub type_url: ::prost::alloc::string::String,
    #[prost(bytes = "vec", tag = "2")]
    pub value: ::prost::alloc::vec::Vec<u8>,
}

impl Any {
    pub fn from_msg<M>(msg: &M) -> Result<Self, EncodeError>
    where
        M: Name,
    {
        let type_url = M::type_url();
        let mut value = Vec::new();
        Message::encode(msg, &mut value)?;
        Ok(Any { type_url, value })
    }

    pub fn to_msg<M>(&self) -> Result<M, DecodeError>
    where
        M: Default + Name + Sized,
    {
        let expected_type_url = M::type_url();

        if let (Some(expected), Some(actual)) = (
            TypeUrl::new(&expected_type_url),
            TypeUrl::new(&self.type_url),
        ) {
            if expected == actual {
                return M::decode(self.value.as_slice());
            }
        }

        let mut err = DecodeError::new(format!(
            "expected type URL: \"{}\" (got: \"{}\")",
            expected_type_url, &self.type_url
        ));
        err.push("unexpected type URL", "type_url");
        Err(err)
    }
}

#[derive(Debug, Eq, PartialEq)]
struct TypeUrl<'a> {
    /// Fully qualified name of the type, e.g. `google.protobuf.Duration`
    pub(crate) full_name: &'a str,
}

impl<'a> TypeUrl<'a> {
    fn new(s: &'a str) -> core::option::Option<Self> {
        let slash_pos = s.rfind('/')?;
        let full_name = s.get((slash_pos + 1)..)?;
        if full_name.starts_with('.') {
            return None;
        }
        Some(Self { full_name })
    }
}
