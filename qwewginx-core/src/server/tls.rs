use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::ServerConfig;
use rustls_pemfile::{certs, pkcs8_private_keys, rsa_private_keys};
use tokio_rustls::TlsAcceptor;

use super::ServerError;

pub struct TlsAcceptorHandle {
    pub inner: TlsAcceptor,
}

impl TlsAcceptorHandle {
    pub fn load(cert_path: &Path, key_path: &Path) -> Result<Arc<Self>, ServerError> {
        let cert = load_certs(cert_path)?;
        let key = load_key(key_path)?;
        let mut config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert, key)
            .map_err(|e| ServerError::Msg(format!("tls config: {e}")))?;
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        Ok(Arc::new(Self {
            inner: TlsAcceptor::from(Arc::new(config)),
        }))
    }
}

fn load_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>, ServerError> {
    let file = File::open(path)
        .map_err(|e| ServerError::Msg(format!("open cert {}: {e}", path.display())))?;
    let mut reader = BufReader::new(file);
    let certs: Vec<_> = certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ServerError::Msg(format!("parse cert {}: {e}", path.display())))?;
    if certs.is_empty() {
        return Err(ServerError::Msg(format!(
            "no certificates in {}",
            path.display()
        )));
    }
    Ok(certs)
}

fn load_key(path: &Path) -> Result<PrivateKeyDer<'static>, ServerError> {
    let file = File::open(path)
        .map_err(|e| ServerError::Msg(format!("open key {}: {e}", path.display())))?;
    let mut reader = BufReader::new(file);
    if let Some(key) = pkcs8_private_keys(&mut reader)
        .next()
        .transpose()
        .map_err(|e| ServerError::Msg(format!("parse key {}: {e}", path.display())))?
    {
        return Ok(PrivateKeyDer::Pkcs8(key));
    }

    let file = File::open(path)
        .map_err(|e| ServerError::Msg(format!("open key {}: {e}", path.display())))?;
    let mut reader = BufReader::new(file);
    if let Some(key) = rsa_private_keys(&mut reader)
        .next()
        .transpose()
        .map_err(|e| ServerError::Msg(format!("parse rsa key {}: {e}", path.display())))?
    {
        return Ok(PrivateKeyDer::Pkcs1(key));
    }

    Err(ServerError::Msg(format!(
        "no private key found in {}",
        path.display()
    )))
}
