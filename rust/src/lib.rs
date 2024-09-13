pub use ::prost;
pub use ::tonic;

pub mod relay {
    tonic::include_proto!("relay");
}

pub mod selfserve {
    pub mod app {
        tonic::include_proto!("selfserve.app");
    }
    pub mod orb {
        tonic::include_proto!("selfserve.orb");
    }
    tonic::include_proto!("selfserve");
}

pub mod config {
    tonic::include_proto!("config");
}
