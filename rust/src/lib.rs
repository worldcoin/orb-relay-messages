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

        impl AppAuthenticatedData {
            /// Returns `true` if `hash` matches this [`AppAuthenticatedData`]
            /// using length-prefixed BLAKE3 hashing.
            ///
            /// Use this for QR v5+ verification.
            pub fn verify_with_length_prefix(&self, hash: impl AsRef<[u8]>) -> bool {
                let external_hash = hash.as_ref();
                if external_hash.is_empty() {
                    return false;
                }
                let internal_hash = self.hash_with_length_prefix(external_hash.len());
                external_hash == internal_hash
            }

            /// Returns `true` if `hash` matches this [`AppAuthenticatedData`]
            /// using the legacy (unfixed) hashing.
            ///
            /// Use this for QR v4 verification. Do not use for new code.
            pub fn verify(&self, hash: impl AsRef<[u8]>) -> bool {
                let external_hash = hash.as_ref();
                if external_hash.is_empty() {
                    return false;
                }
                let internal_hash = self.hash(external_hash.len());
                external_hash == internal_hash
            }

            /// Calculates a BLAKE3 hash of length `n` with length-prefixed fields,
            /// ensuring proper domain separation between variable-length inputs.
            pub fn hash_with_length_prefix(&self, n: usize) -> Vec<u8> {
                let mut hasher = Hasher::new();
                let Self {
                    identity_commitment,
                    self_custody_public_key,
                    pcp_version,
                    os_version,
                    os,
                } = self;
                hasher.update(&(identity_commitment.len() as u32).to_le_bytes());
                hasher.update(identity_commitment.as_bytes());
                hasher.update(&(self_custody_public_key.len() as u32).to_le_bytes());
                hasher.update(self_custody_public_key.as_bytes());
                hasher.update(&(os_version.len() as u32).to_le_bytes());
                hasher.update(os_version.as_bytes());
                hasher.update(&(os.len() as u32).to_le_bytes());
                hasher.update(os.as_bytes());
                hasher.update(&pcp_version.to_le_bytes());
                let mut output = vec![0; n];
                hasher.finalize_xof().fill(&mut output);
                output
            }

            /// Calculates a BLAKE3 hash of length `n` using the legacy (unfixed) method.
            ///
            /// This method concatenates fields without length prefixes, which allows
            /// hash collisions by shifting bytes between adjacent fields.
            /// Kept for backward compatibility with QR v4. Do not use for new code.
            pub fn hash(&self, n: usize) -> Vec<u8> {
                let mut hasher = Hasher::new();
                self.hasher_update(&mut hasher);
                let mut output = vec![0; n];
                hasher.finalize_xof().fill(&mut output);
                output
            }

            fn hasher_update(&self, hasher: &mut Hasher) {
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

    // --- hash_with_length_prefix tests ---

    #[test]
    fn length_prefix_verify_accepts_correct_hash() {
        let data = sample_data();
        let hash = data.hash_with_length_prefix(32);
        assert!(data.verify_with_length_prefix(&hash));
    }

    #[test]
    fn length_prefix_verify_rejects_empty_hash() {
        let data = sample_data();
        assert!(!data.verify_with_length_prefix(b""));
    }

    #[test]
    fn length_prefix_verify_rejects_wrong_hash() {
        let data = sample_data();
        let mut hash = data.hash_with_length_prefix(32);
        hash[0] ^= 0xff;
        assert!(!data.verify_with_length_prefix(&hash));
    }

    #[test]
    fn length_prefix_hash_differs_from_legacy() {
        let data = sample_data();
        assert_ne!(data.hash(32), data.hash_with_length_prefix(32));
    }

    #[test]
    fn length_prefix_prevents_field_boundary_collision() {
        // With the legacy hash, shifting bytes between adjacent fields
        // can produce a collision. With length prefixes, it cannot.
        let data_a = AppAuthenticatedData {
            identity_commitment: "abc".into(),
            self_custody_public_key: "def".into(),
            os: "iOS".into(),
            os_version: "18.0".into(),
            pcp_version: 2,
        };
        let data_b = AppAuthenticatedData {
            identity_commitment: "ab".into(),
            self_custody_public_key: "cdef".into(),
            os: "iOS".into(),
            os_version: "18.0".into(),
            pcp_version: 2,
        };
        // Legacy hash: these collide because "abc"+"def" == "ab"+"cdef"
        assert_eq!(data_a.hash(32), data_b.hash(32));
        // Length-prefixed hash: these do NOT collide
        assert_ne!(
            data_a.hash_with_length_prefix(32),
            data_b.hash_with_length_prefix(32)
        );
    }

    #[test]
    fn length_prefix_pcp_version_always_included() {
        // Unlike legacy hash, pcp_version is always hashed (even when default)
        // so we just verify the hash is stable and different from non-default
        let data_default = sample_data(); // pcp_version = 2
        let mut data_v3 = sample_data();
        data_v3.pcp_version = 3;
        assert_ne!(
            data_default.hash_with_length_prefix(32),
            data_v3.hash_with_length_prefix(32)
        );
    }

    #[test]
    fn legacy_and_length_prefix_are_not_cross_compatible() {
        let data = sample_data();
        let legacy_hash = data.hash(32);
        let lp_hash = data.hash_with_length_prefix(32);
        // Legacy hash should not verify with length-prefix verifier
        assert!(!data.verify_with_length_prefix(&legacy_hash));
        // Length-prefix hash should not verify with legacy verifier
        assert!(!data.verify(&lp_hash));
    }
}
