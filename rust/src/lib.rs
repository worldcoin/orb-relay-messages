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
                if external_hash.is_empty() {
                    return false;
                }
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

#[cfg(test)]
mod tests {
    use super::common::v1::AppAuthenticatedData;

    fn sample_data() -> AppAuthenticatedData {
        AppAuthenticatedData {
            self_custody_public_key: "pk_abc123".into(),
            identity_commitment: "ic_def456".into(),
            os: "iOS".into(),
            os_version: "18.0".into(),
            pcp_version: 2,
        }
    }

    #[test]
    fn verify_rejects_empty_hash() {
        let data = sample_data();
        assert!(!data.verify(b""), "empty hash must not verify");
        assert!(!data.verify(Vec::<u8>::new()), "empty vec must not verify");
    }

    #[test]
    fn verify_accepts_correct_hash() {
        let data = sample_data();
        let hash = data.hash(32);
        assert!(data.verify(&hash));
    }

    #[test]
    fn verify_rejects_wrong_hash() {
        let data = sample_data();
        let mut hash = data.hash(32);
        hash[0] ^= 0xff;
        assert!(!data.verify(&hash));
    }

    #[test]
    fn verify_accepts_shorter_hash_due_to_xof_prefix_consistency() {
        // BLAKE3 XOF is prefix-consistent: first N bytes of hash(M) == hash(N) for N < M
        let data = sample_data();
        let hash = data.hash(32);
        assert!(data.verify(&hash[..16]));
    }

    #[test]
    fn hash_length_matches_requested() {
        let data = sample_data();
        for n in [1, 16, 32, 64] {
            assert_eq!(data.hash(n).len(), n);
        }
    }

    #[test]
    fn different_data_produces_different_hash() {
        let data1 = sample_data();
        let mut data2 = sample_data();
        data2.identity_commitment = "ic_different".into();
        assert_ne!(data1.hash(32), data2.hash(32));
    }

    #[test]
    fn pcp_version_affects_hash_when_non_default() {
        let data_default = sample_data(); // pcp_version = 2 (default)
        let mut data_v3 = sample_data();
        data_v3.pcp_version = 3;
        assert_ne!(data_default.hash(32), data_v3.hash(32));
    }
}
