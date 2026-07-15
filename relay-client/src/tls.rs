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

#[cfg(test)]
mod tests {

    use orb_security_utils::certs::all_pem_certs;
    use rustls::RootCertStore;
    use rustls_pki_types::{pem::PemObject, CertificateDer};

    #[test]
    fn all_pinned_cas_parse_into_root_store() {
        let certs: Vec<CertificateDer> = all_pem_certs()
            .into_iter()
            .map(|pem| {
                CertificateDer::from_pem_slice(pem)
                    .expect("pinned CA must be a valid PEM")
            })
            .collect();

        let mut roots = RootCertStore::empty();
        let (added, ignored) = roots.add_parsable_certificates(certs);

        assert_eq!(0, ignored, "a pinned CA failed DER parsing");
        assert_eq!(added, all_pem_certs().len());
    }
}
