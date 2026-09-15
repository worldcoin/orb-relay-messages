use orb_relay_messages::{
    common::v1::{AnnounceAppId, AnnounceOrbId},
    prost::Message,
    prost_types::Timestamp,
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
        expires_at: Some(Timestamp {
            seconds: 1_725_302_400,
            nanos: 0,
        }),
        request_nonce: vec![7, 8, 9],
        signature: vec![10, 11, 12],
        ..Default::default()
    };
    let app_announcement = AnnounceAppId {
        encrypted_ipcp: vec![13, 14, 15],
        orb_nonce: vec![4, 5, 6],
        integrity_token: "integrity-token".into(),
        integrity_signature: vec![19, 20, 21],
        integrity_timestamp: 1_725_302_400,
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
fn app_announcement_ignores_removed_hash_field() {
    let expected = AnnounceAppId {
        encrypted_ipcp: vec![13, 14, 15],
        orb_nonce: vec![4, 5, 6],
        integrity_token: "integrity-token".into(),
        integrity_signature: vec![19, 20, 21],
        integrity_timestamp: 1_725_302_400,
        ..Default::default()
    };
    let mut legacy_bytes = expected.encode_to_vec();
    legacy_bytes.extend_from_slice(&[0x4a, 0x03, 16, 17, 18]);

    let decoded = AnnounceAppId::decode(legacy_bytes.as_slice()).unwrap();
    assert_eq!(decoded, expected);
    assert_eq!(decoded.encode_to_vec(), expected.encode_to_vec());
}

#[test]
fn app_announcement_preserves_remaining_field_numbers() {
    let announcement = AnnounceAppId {
        encrypted_ipcp: vec![1],
        orb_nonce: vec![2],
        integrity_token: "jwt".into(),
        integrity_signature: vec![3],
        integrity_timestamp: 1,
        ..Default::default()
    };

    assert_eq!(
        announcement.encode_to_vec(),
        [0x42, 1, 1, 0x52, 1, 2, 0x5a, 3, b'j', b'w', b't', 0x62, 1, 3, 0x68, 1]
    );
}
