use orb_qr_link::{decode_qr_with_version, encode_static_qr};
use orb_relay_messages::common::v1::AppAuthenticatedData;
use uuid::Uuid;

const SELF_CUSTODY_PUBLIC_KEY: &str = r#"-----BEGIN PUBLIC KEY-----
MCowBQYDK2VuAyEA2boNBmJX4lGkA9kjthS5crXOBxu2BPycKRMakpzgLG4=
-----END PUBLIC KEY-----"#;

fn make_app_data(identity_commitment: &str, pcp_version: u32) -> AppAuthenticatedData {
    AppAuthenticatedData {
        identity_commitment: identity_commitment.to_string(),
        self_custody_public_key: SELF_CUSTODY_PUBLIC_KEY.to_string(),
        pcp_version,
        os: "Android".to_string(),
        os_version: "1.2.3".to_string(),
        version: AppAuthenticatedData::VERSION,
    }
}

#[test]
fn encode_decode_verifies_app_data() {
    let orb_relay_id = Uuid::new_v4();
    let app_data = make_app_data("0xabcd", 3);
    let hash_app_data = app_data.hash(16);
    let qr = encode_static_qr(&orb_relay_id, hash_app_data);
    let (version, parsed_orb_relay_id, parsed_app_data) =
        decode_qr_with_version(&qr).unwrap();
    assert_eq!(version, 4);
    assert_eq!(parsed_orb_relay_id, orb_relay_id);
    assert!(app_data.verify(parsed_app_data));
}

#[test]
fn encode_decode_rejects_incorrect_app_data() {
    let orb_relay_id = Uuid::new_v4();
    let app_data = make_app_data("0xabcd", 3);
    let hash_app_data = app_data.hash(16);
    let qr = encode_static_qr(&orb_relay_id, hash_app_data);
    let (version, parsed_orb_relay_id, parsed_app_data) =
        decode_qr_with_version(&qr).unwrap();
    assert_eq!(version, 4);
    assert_eq!(parsed_orb_relay_id, orb_relay_id);
    let incorrect_app_data = make_app_data("0x1234", 2);
    assert!(!incorrect_app_data.verify(parsed_app_data));
}
