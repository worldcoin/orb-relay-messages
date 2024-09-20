fn main() {
    assert!(
        cfg!(any(feature = "client", feature = "server")),
        "must specify at least one of the `client` or `server` features"
    );
    tonic_build::configure()
        .build_client(cfg!(feature = "client"))
        .build_server(cfg!(feature = "server"))
        .compile(
            &[
                "../proto/relay.proto",
                "../proto/self_serve/app/v1/app.proto",
                "../proto/self_serve/orb/v1/orb.proto",
                "../proto/config/backend.proto",
                "../proto/config/orb.proto",
            ],
            &["../proto"],
        )
        .unwrap();
}
