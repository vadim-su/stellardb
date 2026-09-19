use std::fmt;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use rustls::ServerConfig;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};

const MAX_CERTIFICATE_FILE_BYTES: u64 = 1024 * 1024;
const MAX_PRIVATE_KEY_FILE_BYTES: u64 = 256 * 1024;

/// A validated certificate chain and private key shared by the HTTP and gRPC listeners.
#[derive(Clone)]
pub struct TlsConfig {
    #[cfg(feature = "grpc")]
    certificate_pem: Arc<[u8]>,
    #[cfg(feature = "grpc")]
    private_key_pem: Arc<[u8]>,
    http_config: axum_server::tls_rustls::RustlsConfig,
}

impl fmt::Debug for TlsConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("TlsConfig").finish_non_exhaustive()
    }
}

impl TlsConfig {
    /// Read and eagerly validate a PEM certificate chain and private key.
    pub fn from_files(
        certificate_path: impl AsRef<Path>,
        private_key_path: impl AsRef<Path>,
    ) -> anyhow::Result<Self> {
        let certificate_pem = read_tls_file(
            certificate_path.as_ref(),
            TlsFileKind::Certificate,
            MAX_CERTIFICATE_FILE_BYTES,
        )?;
        let private_key_pem = read_tls_file(
            private_key_path.as_ref(),
            TlsFileKind::PrivateKey,
            MAX_PRIVATE_KEY_FILE_BYTES,
        )?;
        Self::from_pem(certificate_pem, private_key_pem)
    }

    /// Build TLS configuration from PEM bytes. Primarily useful for ephemeral test identities.
    pub fn from_pem(certificate_pem: Vec<u8>, private_key_pem: Vec<u8>) -> anyhow::Result<Self> {
        let certificates = CertificateDer::pem_slice_iter(&certificate_pem)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| tls_parse_error())?;
        if certificates.is_empty() {
            return Err(tls_parse_error());
        }

        let mut private_keys = PrivateKeyDer::pem_slice_iter(&private_key_pem)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| tls_parse_error())?;
        if private_keys.len() != 1 {
            return Err(tls_parse_error());
        }
        let private_key = private_keys.pop().expect("one TLS private key was checked");

        let mut server_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certificates, private_key)
            .map_err(|_| tls_parse_error())?;
        server_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

        Ok(Self {
            #[cfg(feature = "grpc")]
            certificate_pem: certificate_pem.into(),
            #[cfg(feature = "grpc")]
            private_key_pem: private_key_pem.into(),
            http_config: axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(
                server_config,
            )),
        })
    }

    pub fn http_config(&self) -> axum_server::tls_rustls::RustlsConfig {
        self.http_config.clone()
    }

    #[cfg(feature = "grpc")]
    pub fn grpc_server_config(&self) -> tonic::transport::ServerTlsConfig {
        let identity = tonic::transport::Identity::from_pem(
            self.certificate_pem.as_ref(),
            self.private_key_pem.as_ref(),
        );
        tonic::transport::ServerTlsConfig::new()
            .identity(identity)
            .timeout(std::time::Duration::from_secs(10))
    }
}

#[derive(Clone, Copy)]
enum TlsFileKind {
    Certificate,
    PrivateKey,
}

impl TlsFileKind {
    fn description(self) -> &'static str {
        match self {
            Self::Certificate => "TLS certificate file",
            Self::PrivateKey => "TLS private key file",
        }
    }
}

fn read_tls_file(path: &Path, kind: TlsFileKind, maximum_size: u64) -> anyhow::Result<Vec<u8>> {
    let description = kind.description();
    let path_metadata = std::fs::symlink_metadata(path)
        .map_err(|_| anyhow::anyhow!("failed to inspect {description}"))?;
    if path_metadata.file_type().is_symlink() {
        anyhow::bail!("{description} must not be a symbolic link");
    }
    if !path_metadata.is_file() {
        anyhow::bail!("{description} must be a regular file");
    }

    // This matches the JWT secret-file policy. The post-open check validates the opened target;
    // without a platform-specific O_NOFOLLOW dependency a narrow rename race remains.
    let file =
        std::fs::File::open(path).map_err(|_| anyhow::anyhow!("failed to open {description}"))?;
    let metadata = file
        .metadata()
        .map_err(|_| anyhow::anyhow!("failed to inspect opened {description}"))?;
    if !metadata.is_file() {
        anyhow::bail!("{description} must be a regular file");
    }

    if matches!(kind, TlsFileKind::PrivateKey) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                anyhow::bail!(
                    "TLS private key file must not be accessible by group or other users"
                );
            }
        }
    }

    if metadata.len() > maximum_size {
        anyhow::bail!("{description} is too large");
    }

    let mut bytes = Vec::new();
    file.take(maximum_size + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("failed to read {description}"))?;
    if bytes.len() as u64 > maximum_size {
        anyhow::bail!("{description} is too large");
    }
    Ok(bytes)
}

fn tls_parse_error() -> anyhow::Error {
    anyhow::anyhow!(
        "invalid TLS certificate or private key; provide a PEM certificate chain and one matching unencrypted PKCS#8, PKCS#1, or SEC1 private key"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn make_private(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[cfg(not(unix))]
    fn make_private(_path: &Path) {}

    fn ephemeral_identity() -> (Vec<u8>, Vec<u8>) {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(["localhost".to_string()]).unwrap();
        (
            cert.pem().into_bytes(),
            signing_key.serialize_pem().into_bytes(),
        )
    }

    #[test]
    fn valid_pem_identity_is_accepted() {
        let (certificate, private_key) = ephemeral_identity();
        assert!(TlsConfig::from_pem(certificate, private_key).is_ok());
    }

    #[test]
    fn invalid_or_mismatched_pem_is_rejected_without_leaking_content() {
        let (certificate, _) = ephemeral_identity();
        let (_, other_private_key) = ephemeral_identity();
        let error = TlsConfig::from_pem(certificate, other_private_key).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("invalid TLS certificate or private key")
        );

        let marker = "PRIVATE-CONTENT-MUST-NOT-LEAK";
        let error = TlsConfig::from_pem(marker.as_bytes().to_vec(), marker.as_bytes().to_vec())
            .unwrap_err();
        assert!(!error.to_string().contains(marker));
    }

    #[test]
    fn files_must_be_regular_and_bounded() {
        let directory = tempfile::tempdir().unwrap();
        let (certificate, private_key) = ephemeral_identity();
        let certificate_path = directory.path().join("certificate.pem");
        let private_key_path = directory.path().join("private-key.pem");
        std::fs::write(&certificate_path, certificate).unwrap();
        std::fs::write(&private_key_path, private_key).unwrap();
        make_private(&private_key_path);

        assert!(TlsConfig::from_files(directory.path(), &private_key_path).is_err());
        assert!(TlsConfig::from_files(&certificate_path, directory.path()).is_err());

        let oversized_certificate = directory.path().join("oversized-certificate.pem");
        std::fs::write(
            &oversized_certificate,
            vec![b'x'; MAX_CERTIFICATE_FILE_BYTES as usize + 1],
        )
        .unwrap();
        assert!(TlsConfig::from_files(&oversized_certificate, &private_key_path).is_err());

        let oversized_key = directory.path().join("oversized-private-key.pem");
        std::fs::write(
            &oversized_key,
            vec![b'x'; MAX_PRIVATE_KEY_FILE_BYTES as usize + 1],
        )
        .unwrap();
        make_private(&oversized_key);
        assert!(TlsConfig::from_files(&certificate_path, &oversized_key).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn fifo_is_rejected_without_blocking_on_open() {
        let directory = tempfile::tempdir().unwrap();
        let fifo_path = directory.path().join("certificate.fifo");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo_path)
            .status()
            .unwrap();
        assert!(status.success());

        let (sender, receiver) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            sender
                .send(read_tls_file(
                    &fifo_path,
                    TlsFileKind::Certificate,
                    MAX_CERTIFICATE_FILE_BYTES,
                ))
                .unwrap();
        });
        let error = receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("FIFO validation must return before File::open can block")
            .unwrap_err();
        reader.join().unwrap();
        assert!(error.to_string().contains("must be a regular file"));
    }

    #[cfg(unix)]
    #[test]
    fn private_key_rejects_public_permissions_and_both_files_reject_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let directory = tempfile::tempdir().unwrap();
        let (certificate, private_key) = ephemeral_identity();
        let certificate_path = directory.path().join("certificate.pem");
        let private_key_path = directory.path().join("private-key.pem");
        std::fs::write(&certificate_path, certificate).unwrap();
        std::fs::write(&private_key_path, private_key).unwrap();
        std::fs::set_permissions(&private_key_path, std::fs::Permissions::from_mode(0o644))
            .unwrap();
        assert!(TlsConfig::from_files(&certificate_path, &private_key_path).is_err());

        make_private(&private_key_path);
        assert!(TlsConfig::from_files(&certificate_path, &private_key_path).is_ok());

        let certificate_link = directory.path().join("certificate-link.pem");
        symlink(&certificate_path, &certificate_link).unwrap();
        assert!(TlsConfig::from_files(&certificate_link, &private_key_path).is_err());

        let private_key_link = directory.path().join("private-key-link.pem");
        symlink(&private_key_path, &private_key_link).unwrap();
        assert!(TlsConfig::from_files(&certificate_path, &private_key_link).is_err());
    }
}
