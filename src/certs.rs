use std::io::BufReader;
use std::sync::Arc;
use tokio_rustls::rustls::{Certificate, PrivateKey, ServerConfig};

const CERT: &[u8] = include_bytes!("../certs/server.crt");
const PKEY: &[u8] = include_bytes!("../certs/server.key");

pub type Acceptor = tokio_rustls::TlsAcceptor;

fn tls_acceptor_impl(cert_der: &[u8], key_der: &[u8]) -> Acceptor {
    let mut reader = BufReader::new(cert_der);
    let certs: Vec<_> = rustls_pemfile::certs(&mut reader)
        .into_iter()
        .flatten()
        .map(Certificate)
        .collect();

    let mut reader = BufReader::new(key_der);
    let key = rustls_pemfile::pkcs8_private_keys(&mut reader)
        .into_iter()
        .flatten()
        .map(PrivateKey)
        .next()
        .expect("no private key was found");

    let mut cfg = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();

    cfg.alpn_protocols.push(b"h2".to_vec());
    cfg.alpn_protocols.push(b"http/1.1".to_vec());

    Arc::new(cfg).into()
}

pub fn tls_acceptor() -> Acceptor {
    tls_acceptor_impl(CERT, PKEY)
}
