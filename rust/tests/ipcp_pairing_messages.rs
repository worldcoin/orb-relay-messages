use orb_relay_messages::{
    common::v1::{
        AnnounceAppId, AnnounceOrbId, IpcpHpkePayload, SignedAnnounceOrbId,
        UnsupportedVersion,
    },
    prost::Message,
    self_serve::app::v1::PairingRequest,
};

fn round_trip<M>(message: M) -> M
where
    M: Message + Default,
{
    let encoded = message.encode_to_vec();
    M::decode(encoded.as_slice()).unwrap()
}

#[test]
fn ipcp_pairing_fields_round_trip() {
    let orb_announcement = AnnounceOrbId {
        ipcp_encryption_public_key: vec![1, 2, 3],
        orb_nonce: vec![4, 5, 6],
        request_nonce: vec![7, 8, 9],
        ..Default::default()
    };
    let app_announcement = AnnounceAppId {
        encrypted_ipcp_payload: Some(IpcpHpkePayload {
            enc: vec![13, 14, 15],
            ciphertext: vec![16, 17, 18],
        }),
        orb_nonce: vec![4, 5, 6],
        integrity_token: "integrity-token".into(),
        integrity_signature: vec![19, 20, 21],
        ..Default::default()
    };
    let pairing_request = PairingRequest {
        request_nonce: vec![7, 8, 9],
        ..Default::default()
    };

    assert_eq!(round_trip(orb_announcement.clone()), orb_announcement);
    assert_eq!(round_trip(app_announcement.clone()), app_announcement);
    assert_eq!(round_trip(pairing_request.clone()), pairing_request);
}

#[test]
fn pairing_request_rejects_truncated_request_nonce() {
    assert!(PairingRequest::decode([0x22, 0x80].as_slice()).is_err());
}

#[test]
fn hpke_payload_uses_expected_field_numbers() {
    let payload = IpcpHpkePayload {
        enc: vec![1],
        ciphertext: vec![2, 3],
    };
    let encoded = [0x0a, 1, 1, 0x12, 2, 2, 3];

    assert_eq!(payload.encode_to_vec(), encoded);
    assert_eq!(
        IpcpHpkePayload::decode(encoded.as_slice()).unwrap(),
        payload
    );
}

#[test]
fn app_announcement_rejects_truncated_hpke_fields() {
    for field in [0x0a, 0x12] {
        assert!(AnnounceAppId::decode([0x42, 2, field, 0x80].as_slice()).is_err());
    }
}

#[test]
fn app_announcement_round_trips_without_hpke_payload() {
    let announcement = AnnounceAppId {
        protocol_version: 1,
        heartbeat: true,
        ..Default::default()
    };

    assert!(announcement.encrypted_ipcp_payload.is_none());
    assert_eq!(round_trip(announcement.clone()), announcement);
}

#[test]
fn orb_announcement_ignores_removed_expiry_field() {
    let expected = AnnounceOrbId {
        ipcp_encryption_public_key: vec![1, 2, 3],
        orb_nonce: vec![4, 5, 6],
        request_nonce: vec![7, 8, 9],
        ..Default::default()
    };
    let mut legacy_bytes = expected.encode_to_vec();
    legacy_bytes.extend_from_slice(&[0x72, 0x02, 0x08, 0x01]);

    let decoded = AnnounceOrbId::decode(legacy_bytes.as_slice()).unwrap();
    assert_eq!(decoded, expected);
    assert_eq!(decoded.encode_to_vec(), expected.encode_to_vec());
}

#[test]
fn orb_announcement_preserves_remaining_field_numbers() {
    let announcement = AnnounceOrbId {
        ipcp_encryption_public_key: vec![1],
        orb_nonce: vec![2],
        request_nonce: vec![3],
        ..Default::default()
    };

    assert_eq!(
        announcement.encode_to_vec(),
        [0x62, 1, 1, 0x6a, 1, 2, 0x7a, 1, 3]
    );
}

#[test]
fn orb_announcement_ignores_removed_signature_field() {
    let expected = AnnounceOrbId {
        orb_nonce: vec![4, 5, 6],
        ..Default::default()
    };
    let mut legacy_bytes = expected.encode_to_vec();
    legacy_bytes.extend_from_slice(&[0x82, 1, 2, 9, 9]);

    let decoded = AnnounceOrbId::decode(legacy_bytes.as_slice()).unwrap();
    assert_eq!(decoded, expected);
    assert_eq!(decoded.encode_to_vec(), expected.encode_to_vec());
}

#[test]
fn signed_orb_announcement_uses_expected_field_numbers() {
    let signed = SignedAnnounceOrbId {
        version: 1,
        announcement: vec![2],
        signature: vec![3, 4],
    };
    let encoded = [0x08, 1, 0x12, 1, 2, 0x1a, 2, 3, 4];

    assert_eq!(signed.encode_to_vec(), encoded);
    assert_eq!(
        SignedAnnounceOrbId::decode(encoded.as_slice()).unwrap(),
        signed
    );
}

#[test]
fn signed_orb_announcement_transcript_is_domain_then_announcement() {
    let announcement = AnnounceOrbId {
        orb_id: "ab".into(),
        orb_nonce: vec![4],
        ..Default::default()
    }
    .encode_to_vec();
    let signed = SignedAnnounceOrbId {
        version: SignedAnnounceOrbId::VERSION,
        announcement: announcement.clone(),
        signature: vec![9, 9],
    };

    let mut expected = b"orb-relay/announce-orb-id/v1".to_vec();
    expected.extend_from_slice(&announcement);

    assert_eq!(signed.signing_transcript(), Ok(expected));
}

#[test]
fn signed_orb_announcement_rejects_unknown_versions() {
    for version in [0, 2] {
        let signed = SignedAnnounceOrbId {
            version,
            ..Default::default()
        };

        assert_eq!(
            signed.signing_transcript(),
            Err(UnsupportedVersion(version))
        );
    }
}

#[test]
fn app_announcement_ignores_removed_hash_field() {
    let expected = AnnounceAppId {
        encrypted_ipcp_payload: Some(IpcpHpkePayload {
            enc: vec![13, 14, 15],
            ciphertext: vec![16, 17, 18],
        }),
        orb_nonce: vec![4, 5, 6],
        integrity_token: "integrity-token".into(),
        integrity_signature: vec![19, 20, 21],
        ..Default::default()
    };
    let mut legacy_bytes = expected.encode_to_vec();
    legacy_bytes.extend_from_slice(&[0x4a, 0x03, 16, 17, 18]);

    let decoded = AnnounceAppId::decode(legacy_bytes.as_slice()).unwrap();
    assert_eq!(decoded, expected);
    assert_eq!(decoded.encode_to_vec(), expected.encode_to_vec());
}

#[test]
fn app_announcement_ignores_removed_timestamp_field() {
    let expected = AnnounceAppId {
        encrypted_ipcp_payload: Some(IpcpHpkePayload {
            enc: vec![13, 14, 15],
            ciphertext: vec![16, 17, 18],
        }),
        orb_nonce: vec![4, 5, 6],
        integrity_token: "integrity-token".into(),
        integrity_signature: vec![19, 20, 21],
        ..Default::default()
    };
    let mut legacy_bytes = expected.encode_to_vec();
    legacy_bytes.extend_from_slice(&[0x68, 0x80, 0x8d, 0xd8, 0xb6, 0x06]);

    let decoded = AnnounceAppId::decode(legacy_bytes.as_slice()).unwrap();
    assert_eq!(decoded, expected);
    assert_eq!(decoded.encode_to_vec(), expected.encode_to_vec());
}

#[test]
fn app_announcement_preserves_remaining_field_numbers() {
    let announcement = AnnounceAppId {
        encrypted_ipcp_payload: Some(IpcpHpkePayload {
            enc: vec![1],
            ciphertext: vec![2, 3],
        }),
        orb_nonce: vec![2],
        integrity_token: "jwt".into(),
        integrity_signature: vec![3],
        ..Default::default()
    };

    assert_eq!(
        announcement.encode_to_vec(),
        [
            0x42, 7, 0x0a, 1, 1, 0x12, 2, 2, 3, 0x52, 1, 2, 0x5a, 3, b'j', b'w', b't',
            0x62, 1, 3
        ]
    );
}
