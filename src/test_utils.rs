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

mod images {
    use std::{borrow::Cow, collections::HashMap, env};
    use testcontainers::{core::WaitFor, CopyDataSource, CopyToContainer, Image};

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
        cmd: Vec<String>,
        fsync_enabled: bool,
    }

    impl Postgres {
        /// Set `method` as default auth method for any host/db/user,
        /// it adds the following line to the end of the `pg_hba.conf`:
        /// ```host all all all {method}"```
        pub fn with_auth_method(mut self, method: &str) -> Self {
            self.env_vars
                .insert("POSTGRES_HOST_AUTH_METHOD".to_owned(), method.to_owned());
            self
        }

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

        /// Registers sql to be executed automatically when the container starts.
        /// Can be called multiple times to add (not override) scripts.
        ///
        /// # Example
        ///
        /// ```
        /// # use testcontainers_modules::postgres::Postgres;
        /// let postgres_image = Postgres::default().with_init_sql(
        ///     "CREATE EXTENSION IF NOT EXISTS hstore;"
        ///         .to_string()
        ///         .into_bytes(),
        /// );
        /// ```
        ///
        /// ```rust,ignore
        /// # use testcontainers_modules::postgres::Postgres;
        /// let postgres_image = Postgres::default()
        ///                                .with_init_sql(include_str!("path_to_init.sql").to_string().into_bytes());
        /// ```
        pub fn with_init_sql(mut self, init_sql: impl Into<CopyDataSource>) -> Self {
            let target = format!(
                "/docker-entrypoint-initdb.d/init_{i}.sql",
                i = self.copy_to_sources.len()
            );
            self.copy_to_sources
                .push(CopyToContainer::new(init_sql.into(), target));
            self
        }

        /// Enable SSL on server and copy certificates to config folder
        pub fn with_ssl_enabled(
            mut self,
            ca_cert: impl Into<CopyDataSource>,
            server_cert: impl Into<CopyDataSource>,
            server_key: impl Into<CopyDataSource>,
        ) -> Self {
            const SSL_CMDS: [&str; 8] = [
                "-c",
                "ssl=on",
                "-c",
                "ssl_ca_file=ca.pem",
                "-c",
                "ssl_cert_file=server.crt",
                "-c",
                "ssl_key_file=server.key",
            ];

            self.copy_to_sources.push(CopyToContainer::new(
                ca_cert.into(),
                "/var/lib/postgresql/data/ca.pem".to_owned(),
            ));
            self.copy_to_sources.push(CopyToContainer::new(
                server_cert.into(),
                "/var/lib/postgresql/data/server.crt".to_owned(),
            ));
            self.copy_to_sources.push(CopyToContainer::new(
                server_key.into(),
                "/var/lib/postgresql/data/server.key".to_owned(),
            ));

            SSL_CMDS
                .into_iter()
                .map(String::from)
                .for_each(|s| self.cmd.push(s));

            self
        }

        /// Enables [the fsync-setting](https://www.postgresql.org/docs/current/runtime-config-wal.html#GUC-FSYNC) for the Postgres instance.
        pub fn with_fsync_enabled(mut self) -> Self {
            self.fsync_enabled = true;
            self
        }
    }

    impl Default for Postgres {
        fn default() -> Self {
            let mut env_vars = HashMap::new();
            env_vars.insert("POSTGRES_DB".to_owned(), "postgres".to_owned());
            env_vars.insert("POSTGRES_USER".to_owned(), "postgres".to_owned());
            env_vars.insert("POSTGRES_PASSWORD".to_owned(), "postgres".to_owned());

            Self {
                env_vars,
                copy_to_sources: Vec::new(),
                cmd: Vec::new(),
                fsync_enabled: false,
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
            let mut cmd: Vec<&str> = self.cmd.iter().map(String::as_str).collect();

            if !self.fsync_enabled {
                cmd.push("-c");
                cmd.push("fsync=off");
            }

            cmd
        }
    }
}
