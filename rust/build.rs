fn main() {
    assert!(
        cfg!(any(feature = "client", feature = "server")),
        "must specify at least one of the `client` or `server` features"
    );
    let mut config = prost_build::Config::new();
    config
        .enable_type_names()
        .type_name_domain(["."], "type.googleapis.com")
        .message_attribute(
            ".self_serve.app.v1.AbortSignup",
            "#[derive(serde::Serialize, schemars::JsonSchema)]",
        )
        .enum_attribute(
            ".self_serve.app.v1.AbortSignup.Reason",
            "#[derive(serde::Serialize, schemars::JsonSchema)]",
        );
    tonic_prost_build::configure()
        .build_client(cfg!(feature = "client"))
        .build_server(cfg!(feature = "server"))
        .compile_with_config(
            config,
            &[
                "./proto/relay.proto",
                "./proto/common/v1/orb.proto",
                "./proto/common/v1/misc.proto",
                "./proto/self_serve/app/v1/app.proto",
                "./proto/self_serve/orb/v1/orb.proto",
                "./proto/config/backend.proto",
                "./proto/config/orb.proto",
                "./proto/jobs/jobs.proto",
            ],
            &["./proto", "/usr/local/include"],
        )
        .unwrap();
}
