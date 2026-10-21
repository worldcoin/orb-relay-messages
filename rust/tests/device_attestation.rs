use orb_relay_messages::{
    attestation::v1::DeviceAttestation, prost::Name, prost_types::Any,
};

#[test]
fn device_attestation_round_trips_through_any() {
    let msg = DeviceAttestation {
        payload_json: r#"{"orb_id":"3d22db45"}"#.into(),
    };
    let any = Any::from_msg(&msg).unwrap();
    assert_eq!(
        any.type_url,
        "type.googleapis.com/attestation.v1.DeviceAttestation"
    );
    assert_eq!(any.type_url, DeviceAttestation::type_url());
    assert_eq!(any.to_msg::<DeviceAttestation>().unwrap(), msg);
}
