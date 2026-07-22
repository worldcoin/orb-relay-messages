#[cfg(not(feature = "testing"))]
use orb_security_utils::certs::all_pem_certs;

use tonic::transport::ClientTlsConfig;

#[cfg(not(feature = "testing"))]
use tonic::transport::Certificate;

#[cfg(feature = "testing")]
use orb_relay_test_utils::ca_certificate;

pub fn client_tls_config() -> ClientTlsConfig {
    #[cfg(not(feature = "testing"))]
    {
        all_pem_certs()
            .into_iter()
            .fold(ClientTlsConfig::new(), |config, cert_pem| {
                config.ca_certificate(Certificate::from_pem(cert_pem))
            })
    }

    #[cfg(feature = "testing")]
    {
        ClientTlsConfig::new().ca_certificate(ca_certificate())
    }
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
