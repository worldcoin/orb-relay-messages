pub use ::prost;

pub mod relay {
    tonic::include_proto!("relay");
}

pub mod selfserve {
    tonic::include_proto!("selfserve");
}

pub mod config {
    tonic::include_proto!("config");
}
