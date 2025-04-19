use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};
use std::{
    env,
    error::Error,
    fs::Permissions,
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::atomic::{AtomicU16, Ordering},
};
use testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt};
use tokio::{fs, sync::OnceCell};
use tracing::info;

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

    init_certs("localhost", "exporter", &folder).await
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
        Self::new("localhost", "exporter").unwrap()
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
        let server = server.into();

        // generate root CA key and cert
        let ca_key = KeyPair::generate()?;
        let mut ca_cert = CertificateParams::new(vec![])?;
        ca_cert.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_cert
            .distinguished_name
            .push(DnType::CommonName, "Test Root CA".to_string());
        let ca_cert = ca_cert.self_signed(&ca_key)?;

        // prepare SANs
        let mut hostnames = vec![
            "localhost".to_string(),
            "127.0.0.1".to_string(),
            "::1".to_string(),
        ];
        if server != "localhost" {
            hostnames.insert(0, server.clone());
        }

        // and generate server key and cert
        let server_key = KeyPair::generate()?;
        let mut server_cert_params = CertificateParams::new(hostnames)?;
        server_cert_params
            .distinguished_name
            .push(DnType::CommonName, server);
        let server_cert = server_cert_params.signed_by(&server_key, &ca_cert, &ca_key)?;

        // client part
        let client_key = KeyPair::generate()?;
        let mut client_cert_params = CertificateParams::new(vec![])?;
        client_cert_params
            .distinguished_name
            .push(DnType::CommonName, client.into());
        let client_cert = client_cert_params.signed_by(&client_key, &ca_cert, &ca_key)?;

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

        fs::write(format!("{folder}/server.crt"), &self.server_cert).await?;
        fs::write(format!("{folder}/server.key"), &self.server_key).await?;
        fs::write(format!("{folder}/client.crt"), &self.client_cert).await?;
        fs::write(format!("{folder}/client.key"), &self.client_key).await?;
        fs::write(format!("{folder}/ca.pem"), &self.ca).await?;

        for file in [
            "server.crt",
            "server.key",
            "client.crt",
            "client.key",
            "ca.pem",
        ] {
            fs::set_permissions(format!("{folder}/{file}"), Permissions::from_mode(0o600)).await?;
        }

        Ok(())
    }
}

static CONTAINER: OnceCell<ContainerAsync<images::Postgres>> = OnceCell::const_new();

pub async fn init_psql_server() -> u16 {
    init_tracing().await;
    init_certs("localhost", "exporter", "tests/tls")
        .await
        .unwrap();

    let port = psql_server_container()
        .await
        .get_host_port_ipv4(5432)
        .await
        .unwrap();
    info!(%port, "postgres server started");

    port
}

pub async fn drop_psql_server() {
    let container = psql_server_container().await;
    container.stop().await.unwrap();
}

async fn psql_server_container() -> &'static ContainerAsync<images::Postgres> {
    CONTAINER
        .get_or_init(async || {
            images::Postgres::default()
                .with_db_name("exporter")
                .with_user("exporter")
                .with_password("test-exporter-password")
                .with_init_sql(Path::new("tests/init/init_db.sql"))
                // .with_init_sh(Path::new("tests/init/init_conf.sh"))
                // .with_ssl_enabled()
                .with_container_name("test-psql-query-exporter")
                .start()
                .await
                .unwrap()
        })
        .await
}

mod images {
    use std::{borrow::Cow, collections::HashMap, env};
    use testcontainers::{
        core::{AccessMode, Mount, WaitFor},
        CopyDataSource, CopyToContainer, Image,
    };

    const NAME: &str = "postgres";
    const DEFAULT_PG_VERSION: &str = "17";

    /// Module to work with [`Postgres`] inside of tests.
    ///
    /// Starts an instance of Postgres.
    /// This module is based on the official [`Postgres docker image`].
    ///
    /// Default db name, user and password is `postgres`.
    ///
    /// # Example
    /// ```
    /// use test_utils::{images, testcontainers::runners::SyncRunner};
    ///
    /// let postgres_instance = images::Postgres::default().start().unwrap();
    ///
    /// let connection_string = format!(
    ///     "postgres://postgres:postgres@{}:{}/postgres",
    ///     postgres_instance.get_host().unwrap(),
    ///     postgres_instance.get_host_port_ipv4(5432).unwrap()
    /// );
    /// ```
    #[derive(Debug, Clone)]
    pub struct Postgres {
        env_vars: HashMap<String, String>,
        copy_to_sources: Vec<CopyToContainer>,
        ssl: bool,
        ca_mount: Mount,
        cert_mount: Mount,
        key_mount: Mount,
    }

    impl Postgres {
        /// Sets the db name for the Postgres instance.
        pub fn with_db_name(mut self, db_name: &str) -> Self {
            self.env_vars
                .insert("POSTGRES_DB".to_owned(), db_name.to_owned());
            self
        }

        /// Sets the user for the Postgres instance.
        pub fn with_user(mut self, user: &str) -> Self {
            self.env_vars
                .insert("POSTGRES_USER".to_owned(), user.to_owned());
            self
        }

        /// Sets the password for the Postgres instance.
        pub fn with_password(mut self, password: &str) -> Self {
            self.env_vars
                .insert("POSTGRES_PASSWORD".to_owned(), password.to_owned());
            self
        }

        pub fn with_init_sql(mut self, init_sql: impl Into<CopyDataSource>) -> Self {
            let target = format!(
                "/docker-entrypoint-initdb.d/init_{i}.sql",
                i = self.copy_to_sources.len()
            );
            self.copy_to_sources
                .push(CopyToContainer::new(init_sql.into(), target));
            self
        }

        pub fn with_init_sh(mut self, init_sh: impl Into<CopyDataSource>) -> Self {
            let target = format!(
                "/docker-entrypoint-initdb.d/init_{i}.sh",
                i = self.copy_to_sources.len()
            );
            self.copy_to_sources
                .push(CopyToContainer::new(init_sh.into(), target));
            self
        }

        /// Enable SSL on server and copy certificates to config folder
        pub fn with_ssl_enabled(mut self) -> Self {
            self.ssl = true;
            self
        }
    }

    impl Default for Postgres {
        fn default() -> Self {
            let mut env_vars = HashMap::new();
            env_vars.insert("POSTGRES_DB".to_owned(), "postgres".to_owned());
            env_vars.insert("POSTGRES_USER".to_owned(), "postgres".to_owned());
            env_vars.insert("POSTGRES_PASSWORD".to_owned(), "postgres".to_owned());

            let cargo_folder = env::var("CARGO_MANIFEST_DIR").unwrap();

            Self {
                env_vars,
                copy_to_sources: Vec::new(),
                ssl: false,
                ca_mount: Mount::bind_mount(
                    format!("{cargo_folder}/tests/tls/ca.pem"),
                    "/certs/ca.pem",
                )
                .with_access_mode(AccessMode::ReadOnly),
                cert_mount: Mount::bind_mount(
                    format!("{cargo_folder}/tests/tls/server.crt"),
                    "/certs/server.crt",
                )
                .with_access_mode(AccessMode::ReadOnly),
                key_mount: Mount::bind_mount(
                    format!("{cargo_folder}/tests/tls/server.key"),
                    "/certs/server.key",
                )
                .with_access_mode(AccessMode::ReadOnly),
            }
        }
    }

    impl Image for Postgres {
        fn name(&self) -> &str {
            NAME
        }

        fn tag(&self) -> &str {
            let version = env::var("PG_VERSION").unwrap_or_else(|_| DEFAULT_PG_VERSION.to_owned());
            Box::leak(format!("{version}-alpine").into_boxed_str())
        }

        fn ready_conditions(&self) -> Vec<WaitFor> {
            vec![
                WaitFor::message_on_stderr("database system is ready to accept connections"),
                WaitFor::message_on_stdout("database system is ready to accept connections"),
            ]
        }

        fn env_vars(
            &self,
        ) -> impl IntoIterator<Item = (impl Into<Cow<'_, str>>, impl Into<Cow<'_, str>>)> {
            &self.env_vars
        }

        fn copy_to_sources(&self) -> impl IntoIterator<Item = &CopyToContainer> {
            &self.copy_to_sources
        }

        fn cmd(&self) -> impl IntoIterator<Item = impl Into<std::borrow::Cow<'_, str>>> {
            let mut cmd = vec![];
            if self.ssl {
                cmd.push("-c");
                cmd.push("ssl=on");
                cmd.push("-c");
                cmd.push("ssl_ca_file=/certs/ca.pem");
                cmd.push("-c");
                cmd.push("ssl_cert_file=/certs/server.crt");
                cmd.push("-c");
                cmd.push("ssl_key_file=/certs/server.key");
            }

            cmd
        }

        fn mounts(&self) -> impl IntoIterator<Item = &Mount> {
            if self.ssl {
                vec![&self.ca_mount, &self.cert_mount, &self.key_mount]
            } else {
                vec![]
            }
        }
    }
}

#[tokio::test]
async fn test_start_psql_server() {
    let port = init_psql_server().await;
    assert!(port > 0);
}
