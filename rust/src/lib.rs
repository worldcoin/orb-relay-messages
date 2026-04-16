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

        /// Default PCP version value.
        const PCP_VERSION_DEFAULT: u32 = 2;

        impl AppAuthenticatedData {
            /// Current hash format version for new producers.
            pub const VERSION: u32 = 1;

            /// Returns `true` if `hash` matches this [`AppAuthenticatedData`]
            /// using the current hash format, or the legacy hash format for
            /// app data from clients that do not send `version`.
            ///
            /// `version == 0` is treated as legacy-compatible app data.
            /// `version > 0` only accepts the current length-prefixed format.
            pub fn verify(&self, hash: impl AsRef<[u8]>) -> bool {
                let external_hash = hash.as_ref();
                if external_hash.is_empty() {
                    return false;
                }
                match self.version {
                    Self::VERSION => external_hash == self.hash(external_hash.len()),
                    0 => external_hash == self.legacy_hash(external_hash.len()),
                    _ => false,
                }
            }

            /// Calculates the current length-prefixed BLAKE3 hash of length `n`.
            ///
            /// New producers should set `version` to
            /// [`Self::VERSION`] before serializing app data.
            pub fn hash(&self, n: usize) -> Vec<u8> {
                let mut hasher = Hasher::new();
                let Self {
                    self_custody_public_key,
                    identity_commitment,
                    os,
                    os_version,
                    pcp_version,
                    version,
                } = self;
                assert_eq!(*version, Self::VERSION, "version != Self::VERSION");
                for v in [self_custody_public_key, identity_commitment, os, os_version]
                {
                    let len = u32::try_from(v.len()).expect("less than u32::MAX");
                    hasher.update(&len.to_le_bytes());
                    hasher.update(v.as_bytes());
                }
                hasher.update(&pcp_version.to_le_bytes());
                hasher.update(&version.to_le_bytes());
                let mut output = vec![0; n];
                hasher.finalize_xof().fill(&mut output);
                output
            }

            /// Calculates a BLAKE3 hash of length `n` using the legacy (unfixed) method.
            ///
            /// This method concatenates fields without length prefixes, which allows
            /// hash collisions by shifting bytes between adjacent fields.
            /// Kept for backward compatibility with QR v4. Do not use for new code.
            fn legacy_hash(&self, n: usize) -> Vec<u8> {
                let mut hasher = Hasher::new();
                let Self {
                    identity_commitment,
                    self_custody_public_key,
                    os_version,
                    os,
                    pcp_version,
                    version: _,
                } = self;
                for v in [identity_commitment, self_custody_public_key, os_version, os]
                {
                    hasher.update(v.as_bytes());
                }
                if *pcp_version != PCP_VERSION_DEFAULT {
                    hasher.update(&pcp_version.to_le_bytes());
                }
                let mut output = vec![0; n];
                hasher.finalize_xof().fill(&mut output);
                output
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
    use blake3::Hasher;

    fn app_data(version: u32) -> AppAuthenticatedData {
        AppAuthenticatedData {
            self_custody_public_key: "pk_abc123".into(),
            identity_commitment: "ic_def456".into(),
            os: "iOS".into(),
            os_version: "18.0".into(),
            pcp_version: 2,
            version,
        }
    }

    fn finish(hasher: Hasher, n: usize) -> Vec<u8> {
        let mut output = vec![0; n];
        hasher.finalize_xof().fill(&mut output);
        output
    }

    fn legacy_hash(data: &AppAuthenticatedData, n: usize) -> Vec<u8> {
        let mut hasher = Hasher::new();
        for v in [
            &data.identity_commitment,
            &data.self_custody_public_key,
            &data.os_version,
            &data.os,
        ] {
            hasher.update(v.as_bytes());
        }
        if data.pcp_version != 2 {
            hasher.update(&data.pcp_version.to_le_bytes());
        }
        finish(hasher, n)
    }

    fn current_hash(data: &AppAuthenticatedData, n: usize) -> Vec<u8> {
        let mut hasher = Hasher::new();
        for v in [
            &data.self_custody_public_key,
            &data.identity_commitment,
            &data.os,
            &data.os_version,
        ] {
            let len = u32::try_from(v.len()).unwrap();
            hasher.update(&len.to_le_bytes());
            hasher.update(v.as_bytes());
        }
        hasher.update(&data.pcp_version.to_le_bytes());
        hasher.update(&data.version.to_le_bytes());
        finish(hasher, n)
    }

    #[test]
    fn hash_uses_current_format() {
        let data = app_data(AppAuthenticatedData::VERSION);
        assert_eq!(data.hash(16), current_hash(&data, 16));
        assert_ne!(data.hash(16), legacy_hash(&data, 16));
    }

    #[test]
    #[should_panic]
    fn hash_rejects_missing_version() {
        app_data(0).hash(16);
    }

    #[test]
    fn verify_accepts_current_hash_and_rejects_bad_hashes() {
        let data = app_data(AppAuthenticatedData::VERSION);
        let mut hash = data.hash(16);

        assert!(data.verify(&hash));
        assert!(!data.verify(b""));

        hash[0] ^= 0xff;
        assert!(!data.verify(hash));
    }

    #[test]
    fn verify_accepts_legacy_hash_only_when_version_is_missing() {
        let legacy_data = app_data(0);
        let mut versioned_data = legacy_data.clone();
        versioned_data.version = AppAuthenticatedData::VERSION;
        let hash = legacy_hash(&legacy_data, 16);

        assert!(legacy_data.verify(&hash));
        assert!(!versioned_data.verify(hash));
    }

    #[test]
    fn length_prefix_prevents_field_boundary_collision() {
        let mut data_a = app_data(AppAuthenticatedData::VERSION);
        data_a.identity_commitment = "abc".into();
        data_a.self_custody_public_key = "def".into();

        let mut data_b = data_a.clone();
        data_b.identity_commitment = "ab".into();
        data_b.self_custody_public_key = "cdef".into();

        assert_eq!(legacy_hash(&data_a, 16), legacy_hash(&data_b, 16));
        assert_ne!(data_a.hash(16), data_b.hash(16));
    }
}
