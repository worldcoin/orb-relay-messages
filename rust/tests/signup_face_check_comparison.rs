use orb_relay_messages::{
    prost::Message,
    self_serve::orb::v1::{signup_ended::FaceCheckComparison, SignupEnded},
};

#[test]
fn legacy_signup_success_has_unspecified_face_comparison() {
    let message = SignupEnded::decode([0x08, 0x01].as_slice()).unwrap();

    assert!(message.success);
    assert_eq!(
        message.face_check_comparison(),
        FaceCheckComparison::Unspecified
    );
}

#[test]
fn face_comparison_result_does_not_change_signup_success() {
    for result in [
        FaceCheckComparison::Success,
        FaceCheckComparison::Failure,
        FaceCheckComparison::NotRun,
    ] {
        let message = SignupEnded {
            success: true,
            face_check_comparison: result.into(),
            ..Default::default()
        };
        let encoded = message.encode_to_vec();
        let decoded = SignupEnded::decode(encoded.as_slice()).unwrap();

        assert!(decoded.success);
        assert_eq!(decoded.face_check_comparison(), result);
        assert_eq!(encoded, [0x08, 0x01, 0x18, result as u8]);
    }
}
