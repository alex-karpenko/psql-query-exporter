use std::{
    net::SocketAddr,
    path::Path,
    sync::atomic::{AtomicU16, Ordering},
};
use testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt};
use tokio::sync::OnceCell;
use tracing::info;

use crate::db::{PostgresConnectionString, PostgresSslMode};

pub const TEST_DB_NAME: &str = "exporter";
pub const TEST_DB_USER: &str = "exporter";
pub const TEST_DB_PASSWORD: &str = "test-exporter-password";

pub const TEST_CA_CERT: &str = "tests/tls/ca.pem";
pub const TEST_CLIENT_CERT: &str = "tests/tls/client.crt";
pub const TEST_CLIENT_KEY: &str = "tests/tls/client.key";
pub const TEST_SERVER_CERT: &str = "tests/tls/server.crt";
pub const TEST_SERVER_KEY: &str = "tests/tls/server.key";

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

static PSQL_CONTAINER: OnceCell<ContainerAsync<images::Postgres>> = OnceCell::const_new();

pub async fn init_psql_server() -> u16 {
    init_tracing().await;

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
    PSQL_CONTAINER
        .get_or_init(async || {
            images::Postgres::default()
                .with_db_name(TEST_DB_NAME)
                .with_user(TEST_DB_USER)
                .with_password(TEST_DB_PASSWORD)
                .with_init_sql(Path::new("tests/init/init_db.sql"))
                .with_init_sh(Path::new("tests/init/init_conf.sh"))
                .with_ssl_enabled()
                .with_container_name(format!(
                    "test-psql-query-exporter-v{}",
                    images::Postgres::version()
                ))
                .start()
                .await
                .unwrap()
        })
        .await
}

pub async fn create_test_connection_string(sslmode: PostgresSslMode) -> PostgresConnectionString {
    init_tracing().await;
    let port = init_psql_server().await;

    PostgresConnectionString {
        host: "localhost".to_string(),
        port,
        dbname: TEST_DB_NAME.to_string(),
        user: TEST_DB_USER.to_string(),
        password: TEST_DB_PASSWORD.to_string(),
        sslmode,
    }
}

mod images {
    use super::*;
    use std::{borrow::Cow, collections::HashMap, env};
    use testcontainers::{
        core::{AccessMode, Mount, WaitFor},
        CopyDataSource, CopyToContainer, Image,
    };

    const NAME: &str = "postgres";
    const DEFAULT_PG_VERSION: &str = "17";

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

        pub fn version() -> String {
            env::var("PG_VERSION").unwrap_or_else(|_| DEFAULT_PG_VERSION.to_owned())
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
                    format!("{cargo_folder}/{TEST_CA_CERT}"),
                    "/certs/ca.pem",
                )
                .with_access_mode(AccessMode::ReadOnly),
                cert_mount: Mount::bind_mount(
                    format!("{cargo_folder}/{TEST_SERVER_CERT}"),
                    "/certs/server.crt",
                )
                .with_access_mode(AccessMode::ReadOnly),
                key_mount: Mount::bind_mount(
                    format!("{cargo_folder}/{TEST_SERVER_KEY}"),
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
            Box::leak(format!("{}-alpine", Self::version()).into_boxed_str())
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
