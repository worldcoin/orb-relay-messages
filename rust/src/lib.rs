pub use ::prost;
pub use ::tonic;

pub mod self_serve {
    pub mod relay {
        pub mod v1 {
            tonic::include_proto!("self_serve.relay.v1");
        }
    }
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
