pub use ::prost;
pub use ::prost_types;
pub use ::tonic;

pub mod relay {
    tonic::include_proto!("relay");

    impl From<RelayPayload> for RelayConnectRequest {
        fn from(payload: RelayPayload) -> Self {
            RelayConnectRequest {
                msg: Some(relay_connect_request::Msg::Payload(payload)),
            }
        }
    }

    impl From<Heartbeat> for RelayConnectRequest {
        fn from(heartbeat: Heartbeat) -> Self {
            RelayConnectRequest {
                msg: Some(relay_connect_request::Msg::Heartbeat(heartbeat)),
            }
        }
    }

    impl From<ConnectRequest> for RelayConnectRequest {
        fn from(connect_request: ConnectRequest) -> Self {
            RelayConnectRequest {
                msg: Some(relay_connect_request::Msg::ConnectRequest(connect_request)),
            }
        }
    }
}

pub mod common {
    pub mod v1 {
        tonic::include_proto!("common.v1");

        use blake3::Hasher;

        /// Default PCP version value (proto3 default for uint32)
        const PCP_VERSION_DEFAULT: u32 = 2;

        /// Extension methods for [`AppAuthenticatedData`] that provide hashing and verification functionality.
        impl AppAuthenticatedData {
            /// Returns `true` if `hash` is a BLAKE3 hash of this [`AppAuthenticatedData`].
            ///
            /// This method calculates its own hash of the same length as the input
            /// `hash` and checks if both hashes are identical.
            pub fn verify(&self, hash: impl AsRef<[u8]>) -> bool {
                let external_hash = hash.as_ref();
                let internal_hash = self.hash(external_hash.len());
                external_hash == internal_hash
            }

            /// Calculates a BLAKE3 hash of the length `n`.
            pub fn hash(&self, n: usize) -> Vec<u8> {
                let mut hasher = Hasher::new();
                self.hasher_update(&mut hasher);
                let mut output = vec![0; n];
                hasher.finalize_xof().fill(&mut output);
                output
            }

            /// Updates the provided BLAKE3 hasher with all fields of this [`AppAuthenticatedData`].
            ///
            /// This method must hash every field in the struct to ensure complete validation.
            pub fn hasher_update(&self, hasher: &mut Hasher) {
                let Self {
                    identity_commitment,
                    self_custody_public_key,
                    pcp_version,
                    os_version,
                    os,
                } = self;
                hasher.update(identity_commitment.as_bytes());
                hasher.update(self_custody_public_key.as_bytes());
                hasher.update(os_version.as_bytes());
                hasher.update(os.as_bytes());
                if *pcp_version != PCP_VERSION_DEFAULT {
                    hasher.update(&pcp_version.to_le_bytes());
                }
            }
        }
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

pub mod jobs {
    pub mod v1 {
        tonic::include_proto!("jobs.v1");
    }
}
