use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};
use std::{
    env,
    error::Error,
    fs::Permissions,
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    sync::atomic::{AtomicU16, Ordering},
};
use tokio::{fs, sync::OnceCell};

pub fn next_addr() -> SocketAddr {
    static PORT: AtomicU16 = AtomicU16::new(9000);

    let next_port = PORT.fetch_add(1, Ordering::SeqCst);
    format!("127.0.0.1:{next_port}").parse().unwrap()
}

pub async fn init_tracing() {
    static INIT: OnceCell<()> = OnceCell::const_new();

    INIT.get_or_init(async || tracing_subscriber::fmt::try_init().unwrap())
        .await;
}

pub async fn default_certs(folder: Option<String>) -> Result<&'static String, Box<dyn Error>> {
    let folder = folder.unwrap_or_else(|| {
        env::var("OUT_DIR").expect("OUT_DIR environment variable is not defined")
    });

    init_certs("localhost", "client", &folder).await
}

pub async fn init_certs(
    server: &str,
    client: &str,
    folder: &str,
) -> Result<&'static String, Box<dyn Error>> {
    static INIT: OnceCell<String> = OnceCell::const_new();

    Ok(INIT
        .get_or_init(|| async {
            let certs = TestTlsCerts::new(server, client).unwrap();
            certs.store_to(folder).await.unwrap();
            folder.to_string()
        })
        .await)
}

/// Helper struct to store TLS certificates.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TestTlsCerts {
    pub server_cert: String,
    pub server_key: String,
    pub client_cert: String,
    pub client_key: String,
    pub ca: String,
}

impl Default for TestTlsCerts {
    fn default() -> Self {
        Self::new("localhost", "client").unwrap()
    }
}

impl TestTlsCerts {
    /// Generate new self-signed Root CA certificate,
    /// server and client certificates signed by CA.
    ///
    /// SAN list includes "localhost", "127.0.0.1", "::1"
    /// and provided server hostname (if it's different form localhost).
    pub fn new(
        server: impl Into<String>,
        client: impl Into<String>,
    ) -> Result<Self, Box<dyn Error>> {
        // generate root CA key and cert
        let ca_key = KeyPair::generate()?;
        let mut ca_cert = CertificateParams::new(vec!["Test Root CA".to_string()])?;
        ca_cert.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca_cert = ca_cert.self_signed(&ca_key)?;

        // prepare SANs
        let mut hostnames = vec![
            "localhost".to_string(),
            "127.0.0.1".to_string(),
            "::1".to_string(),
        ];
        let hostname = server.into();
        if hostname != "localhost" {
            hostnames.insert(0, hostname);
        }

        // and generate server key and cert
        let server_key = KeyPair::generate()?;
        let server_cert =
            CertificateParams::new(hostnames)?.signed_by(&server_key, &ca_cert, &ca_key)?;

        // client part
        let client_key = KeyPair::generate()?;
        let client_cert = CertificateParams::new(vec![client.into()])?.signed_by(
            &client_key,
            &ca_cert,
            &ca_key,
        )?;

        Ok(Self {
            server_cert: server_cert.pem(),
            server_key: server_key.serialize_pem(),
            client_cert: client_cert.pem(),
            client_key: client_key.serialize_pem(),
            ca: ca_cert.pem(),
        })
    }

    /// Stores all certificates to the provided folder, with pre-defined names.
    pub async fn store_to(&self, folder: &str) -> Result<(), Box<dyn Error>> {
        fs::create_dir_all(folder).await?;

        fs::write(format!("{folder}/server.pem"), &self.server_cert).await?;
        fs::write(format!("{folder}/server.key"), &self.server_key).await?;
        fs::write(format!("{folder}/client.pem"), &self.client_cert).await?;
        fs::write(format!("{folder}/client.key"), &self.client_key).await?;
        fs::write(format!("{folder}/ca.pem"), &self.ca).await?;

        for file in [
            "server.pem",
            "server.key",
            "client.pem",
            "client.key",
            "ca.pem",
        ] {
            fs::set_permissions(format!("{folder}/{file}"), Permissions::from_mode(0o600)).await?;
        }

        Ok(())
    }
}
