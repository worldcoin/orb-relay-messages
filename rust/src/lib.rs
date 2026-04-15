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

        /// Version prefix for self-describing hash format (v5).
        const HASH_VERSION_V5: u8 = 0x05;

        impl AppAuthenticatedData {
            /// Returns `true` if `hash` matches this [`AppAuthenticatedData`].
            ///
            /// Dispatches based on the hash format:
            /// - `0x05` prefix → try v5 (length-prefixed BLAKE3) first
            /// - falls back to v4 legacy (plain BLAKE3)
            ///
            /// The fallback handles the case where a legacy hash happens to
            /// start with `0x05` (~0.4% probability for random BLAKE3 output).
            pub fn verify(&self, hash: impl AsRef<[u8]>) -> bool {
                let hash = hash.as_ref();
                if hash.is_empty() {
                    return false;
                }
                if hash.first() == Some(&HASH_VERSION_V5)
                    && self.verify_with_length_prefix(&hash[1..])
                {
                    return true;
                }
                self.verify_legacy(hash)
            }

            /// Returns `true` if `hash` matches using length-prefixed BLAKE3.
            ///
            /// Prefer [`Self::verify()`] which handles version dispatch
            /// automatically.
            pub fn verify_with_length_prefix(&self, hash: impl AsRef<[u8]>) -> bool {
                let external_hash = hash.as_ref();
                if external_hash.is_empty() {
                    return false;
                }
                let internal_hash = self.hash_with_length_prefix(external_hash.len());
                external_hash == internal_hash
            }

            /// Returns `true` if `hash` matches using legacy (unfixed) hashing.
            ///
            /// For QR v4 backward compatibility only. Do not use for new code.
            pub fn verify_legacy(&self, hash: impl AsRef<[u8]>) -> bool {
                let external_hash = hash.as_ref();
                if external_hash.is_empty() {
                    return false;
                }
                let internal_hash = self.hash_legacy(external_hash.len());
                external_hash == internal_hash
            }

            /// Calculates a self-describing BLAKE3 hash with version prefix.
            ///
            /// Returns `0x05 ++ BLAKE3(length-prefixed fields)` where the BLAKE3
            /// digest is truncated to `truncate_to` bytes. The total output
            /// length is `truncate_to + 1`.
            pub fn hash(&self, truncate_to: usize) -> Vec<u8> {
                let digest = self.hash_with_length_prefix(truncate_to);
                let mut out = Vec::with_capacity(1 + digest.len());
                out.push(HASH_VERSION_V5);
                out.extend_from_slice(&digest);
                out
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

            /// Calculates a BLAKE3 hash of length `n` using the legacy (unfixed)
            /// method.
            ///
            /// This method concatenates fields without length prefixes, which
            /// allows hash collisions by shifting bytes between adjacent fields.
            /// Kept for backward compatibility with QR v4. Do not use for new
            /// code.
            pub fn hash_legacy(&self, n: usize) -> Vec<u8> {
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

    // --- Self-describing verify() dispatcher tests ---

    #[test]
    fn verify_rejects_empty_hash() {
        let data = sample_data();
        assert!(!data.verify(b""), "empty hash must not verify");
        assert!(!data.verify(Vec::<u8>::new()), "empty vec must not verify");
    }

    #[test]
    fn verify_accepts_self_describing_hash() {
        let data = sample_data();
        let hash = data.hash(32);
        assert!(data.verify(&hash));
    }

    #[test]
    fn verify_accepts_legacy_hash() {
        let data = sample_data();
        let hash = data.hash_legacy(32);
        assert!(data.verify(&hash));
    }

    #[test]
    fn verify_rejects_wrong_self_describing_hash() {
        let data = sample_data();
        let mut hash = data.hash(32);
        // Corrupt a digest byte (not the version prefix)
        hash[1] ^= 0xff;
        assert!(!data.verify(&hash));
    }

    #[test]
    fn verify_rejects_wrong_legacy_hash() {
        let data = sample_data();
        let mut hash = data.hash_legacy(32);
        hash[0] ^= 0xff;
        assert!(!data.verify(&hash));
    }

    #[test]
    fn verify_accepts_legacy_hash_starting_with_version_byte() {
        // ~0.4% of legacy hashes start with 0x05 by chance. The dispatcher
        // tries v5 first (which fails), then falls back to legacy.
        // Brute-force an input whose legacy hash starts with 0x05.
        let data = (0u32..)
            .map(|i| AppAuthenticatedData {
                identity_commitment: format!("ic_{i}"),
                ..sample_data()
            })
            .find(|d| d.hash_legacy(32)[0] == 0x05)
            .expect("should find a collision within a few hundred tries");
        let hash = data.hash_legacy(32);
        assert_eq!(hash[0], 0x05);
        assert!(
            data.verify(&hash),
            "legacy hash starting with 0x05 must still verify"
        );
    }

    // --- Self-describing hash() tests ---

    #[test]
    fn hash_has_version_prefix() {
        let data = sample_data();
        let hash = data.hash(32);
        assert_eq!(hash[0], 0x05, "first byte must be version prefix");
    }

    #[test]
    fn hash_length_is_digest_plus_one() {
        let data = sample_data();
        for n in [1, 16, 32, 64] {
            assert_eq!(data.hash(n).len(), n + 1);
        }
    }

    #[test]
    fn hash_digest_matches_hash_with_length_prefix() {
        let data = sample_data();
        let self_describing = data.hash(32);
        let raw = data.hash_with_length_prefix(32);
        assert_eq!(&self_describing[1..], &raw[..]);
    }

    #[test]
    fn different_data_produces_different_hash() {
        let data1 = sample_data();
        let mut data2 = sample_data();
        data2.identity_commitment = "ic_different".into();
        assert_ne!(data1.hash(32), data2.hash(32));
    }

    // --- Legacy hash tests ---

    #[test]
    fn legacy_verify_rejects_empty_hash() {
        let data = sample_data();
        assert!(!data.verify_legacy(b""), "empty hash must not verify");
        assert!(
            !data.verify_legacy(Vec::<u8>::new()),
            "empty vec must not verify"
        );
    }

    #[test]
    fn legacy_verify_accepts_correct_hash() {
        let data = sample_data();
        let hash = data.hash_legacy(32);
        assert!(data.verify_legacy(&hash));
    }

    #[test]
    fn legacy_verify_rejects_wrong_hash() {
        let data = sample_data();
        let mut hash = data.hash_legacy(32);
        hash[0] ^= 0xff;
        assert!(!data.verify_legacy(&hash));
    }

    #[test]
    fn legacy_verify_accepts_shorter_hash_due_to_xof_prefix_consistency() {
        // BLAKE3 XOF is prefix-consistent: first N bytes of hash(M) == hash(N) for N < M
        let data = sample_data();
        let hash = data.hash_legacy(32);
        assert!(data.verify_legacy(&hash[..16]));
    }

    #[test]
    fn legacy_hash_length_matches_requested() {
        let data = sample_data();
        for n in [1, 16, 32, 64] {
            assert_eq!(data.hash_legacy(n).len(), n);
        }
    }

    #[test]
    fn legacy_pcp_version_affects_hash_when_non_default() {
        let data_default = sample_data(); // pcp_version = 2 (default)
        let mut data_v3 = sample_data();
        data_v3.pcp_version = 3;
        assert_ne!(data_default.hash_legacy(32), data_v3.hash_legacy(32));
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
        assert_ne!(data.hash_legacy(32), data.hash_with_length_prefix(32));
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
        assert_eq!(data_a.hash_legacy(32), data_b.hash_legacy(32));
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
        let legacy_hash = data.hash_legacy(32);
        let lp_hash = data.hash_with_length_prefix(32);
        // Legacy hash should not verify with length-prefix verifier
        assert!(!data.verify_with_length_prefix(&legacy_hash));
        // Length-prefix hash should not verify with legacy verifier
        assert!(!data.verify_legacy(&lp_hash));
    }
}
