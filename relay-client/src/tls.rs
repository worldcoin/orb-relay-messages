use orb_security_utils::certs::all_pem_certs;
use tonic::transport::{Certificate, ClientTlsConfig};

pub fn client_tls_config(additional_root_ca: Option<&str>) -> ClientTlsConfig {
    all_pem_certs()
        .into_iter()
        .chain(additional_root_ca.map(str::as_bytes))
        .fold(ClientTlsConfig::new(), |config, cert_pem| {
            config.ca_certificate(Certificate::from_pem(cert_pem))
        })
}
