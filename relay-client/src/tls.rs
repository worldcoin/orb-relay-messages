use orb_security_utils::reqwest::{
    AWS_ROOT_CA1_CERT, AWS_ROOT_CA2_CERT, AWS_ROOT_CA3_CERT, AWS_ROOT_CA4_CERT,
    GTS_ROOT_R1_CERT, GTS_ROOT_R2_CERT, GTS_ROOT_R3_CERT, GTS_ROOT_R4_CERT,
    SFS_ROOT_G2_CERT,
};
use tonic::transport::{Certificate, ClientTlsConfig};

pub fn client_tls_config(domain_name: &str) -> ClientTlsConfig {
    [
        AWS_ROOT_CA1_CERT,
        AWS_ROOT_CA2_CERT,
        AWS_ROOT_CA3_CERT,
        AWS_ROOT_CA4_CERT,
        SFS_ROOT_G2_CERT,
        GTS_ROOT_R1_CERT,
        GTS_ROOT_R2_CERT,
        GTS_ROOT_R3_CERT,
        GTS_ROOT_R4_CERT,
    ]
    .into_iter()
    .fold(
        ClientTlsConfig::new().domain_name(domain_name.to_owned()),
        |config, cert_pem| config.ca_certificate(Certificate::from_pem(cert_pem)),
    )
}
