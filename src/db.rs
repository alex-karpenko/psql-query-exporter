use crate::{
    errors::PsqlExporterError,
    utils::{ShutdownReceiver, SleepHelper},
};
use openssl::ssl::{SslConnector, SslFiletype, SslMethod, SslVerifyMode};
use postgres_openssl::MakeTlsConnector;
use serde::Deserialize;
use std::{
    fmt::{Debug, Display},
    time::Duration,
};
use tokio::task::JoinHandle;
use tokio_postgres::{Client, Row};
use tracing::{debug, error, instrument};

const DB_APP_NAME: &str = env!("CARGO_PKG_NAME");
const DB_APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct PostgresConnectionString {
    pub host: String,
    pub port: u16,
    pub dbname: String,
    pub user: String,
    pub password: String,
    pub sslmode: PostgresSslMode,
}

impl Display for PostgresConnectionString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "host={host} port={port} dbname={dbname} user={user} password='***' sslmode={sslmode} application_name={DB_APP_NAME}-v{DB_APP_VERSION}", host=self.host, port=self.port, user=self.user, sslmode=self.sslmode, dbname=self.dbname)
    }
}

impl Debug for PostgresConnectionString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "host={host} port={port} dbname={dbname} user={user} password='***' sslmode={sslmode} application_name={DB_APP_NAME}-v{DB_APP_VERSION}", host=self.host, port=self.port, user=self.user, sslmode=self.sslmode, dbname=self.dbname)
    }
}

impl Default for PostgresConnectionString {
    fn default() -> Self {
        PostgresConnectionString {
            host: String::new(),
            port: 5432,
            dbname: String::new(),
            user: String::new(),
            password: String::new(),
            sslmode: PostgresSslMode::Prefer,
        }
    }
}

impl PostgresConnectionString {
    fn get_conn_string(&self) -> String {
        format!("host={host} port={port} dbname={dbname} user={user} password='{password}' sslmode={sslmode} application_name={DB_APP_NAME}-v{DB_APP_VERSION}", host=self.host, port=self.port, user=self.user, password=self.password, sslmode=self.sslmode, dbname=self.dbname)
    }
}
#[derive(Debug)]
pub struct PostgresConnection {
    db_connection_string: PostgresConnectionString,
    client: Client,
    connection_handler: JoinHandle<()>,
    sslmode: PostgresSslMode,
    certificates: PostgresSslCertificates,
    default_backoff_interval: Duration,
    max_backoff_interval: Duration,
    shutdown_channel: ShutdownReceiver,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "lowercase")]
pub enum PostgresSslMode {
    Disable,
    Prefer,
    Require,
    #[serde(rename = "verify-ca")]
    VerifyCa,
    #[serde(rename = "verify-full")]
    VerifyFull,
}

impl Default for PostgresSslMode {
    fn default() -> Self {
        Self::Prefer
    }
}

impl Display for PostgresSslMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Disable => "disable",
            Self::Prefer => "prefer",
            Self::Require => "require",
            Self::VerifyCa => "require",
            Self::VerifyFull => "require",
        };
        write!(f, "{s}")
    }
}

#[derive(Debug, Clone)]
pub struct PostgresSslCertificates {
    rootcert: Option<String>,
    cert: Option<String>,
    key: Option<String>,
}

impl PostgresSslCertificates {
    pub fn from(
        rootcert: Option<String>,
        cert: Option<String>,
        key: Option<String>,
    ) -> Result<Self, PsqlExporterError> {
        match (cert, key) {
            (Some(cert), None) => Err(PsqlExporterError::PostgresTlsClientConfig(format!(
                "private key for client certificate {} should be defined.",
                cert
            ))),
            (None, Some(key)) => Err(PsqlExporterError::PostgresTlsClientConfig(format!(
                "client certificate for private key {} should be defined.",
                key
            ))),
            (Some(cert), Some(key)) => Ok(Self {
                rootcert,
                cert: Some(cert),
                key: Some(key),
            }),
            (None, None) => Ok(Self {
                rootcert,
                cert: None,
                key: None,
            }),
        }
    }

    pub fn has_client_cert(&self) -> bool {
        self.cert.is_some()
    }
}

impl PostgresConnection {
    #[instrument("NewDbConnection", skip_all)]
    pub async fn new(
        db_connection_string: PostgresConnectionString,
        sslmode: PostgresSslMode,
        certificates: PostgresSslCertificates,
        default_backoff_interval: Duration,
        max_backoff_interval: Duration,
        shutdown_channel: ShutdownReceiver,
    ) -> Result<Self, PsqlExporterError> {
        debug!("create new");

        let mut backoff_interval = default_backoff_interval;
        let mut sleeper = SleepHelper::from(shutdown_channel.clone());

        loop {
            let connector = Self::build_tls_connector(&sslmode, &certificates)?;
            let connection =
                tokio_postgres::connect(&db_connection_string.get_conn_string(), connector).await;

            match connection {
                Ok((client, connection)) => {
                    let connection_handler = tokio::spawn(async move {
                        debug!("spawn new connection task");
                        if let Err(e) = connection.await {
                            error!(error = %e);
                        }
                    });

                    return Ok(PostgresConnection {
                        client,
                        db_connection_string,
                        connection_handler,
                        sslmode,
                        certificates,
                        default_backoff_interval,
                        max_backoff_interval,
                        shutdown_channel,
                    });
                }
                Err(e) => {
                    error!(error = %e);
                }
            };

            sleeper.sleep(backoff_interval).await?;
            backoff_interval += default_backoff_interval;
            if backoff_interval > max_backoff_interval {
                backoff_interval = max_backoff_interval;
            }
        }
    }

    #[instrument("BuildTlsConnector", skip_all, fields(sslmode))]
    fn build_tls_connector(
        sslmode: &PostgresSslMode,
        certificates: &PostgresSslCertificates,
    ) -> Result<MakeTlsConnector, PsqlExporterError> {
        let mut connector = SslConnector::builder(SslMethod::tls())
            .map_err(PsqlExporterError::PostgresTlsConnector)?;

        match *sslmode {
            PostgresSslMode::Disable => connector.set_verify(SslVerifyMode::NONE),
            PostgresSslMode::Prefer => connector.set_verify(SslVerifyMode::NONE),
            PostgresSslMode::Require => connector.set_verify(SslVerifyMode::NONE),
            PostgresSslMode::VerifyCa => {
                connector.set_verify_callback(
                    SslVerifyMode::PEER,
                    |verify_indicator, x509_result| {
                        let allowed_errors: Vec<i32> = vec![
                            openssl_sys::X509_V_ERR_IP_ADDRESS_MISMATCH,
                            openssl_sys::X509_V_ERR_HOSTNAME_MISMATCH,
                            openssl_sys::X509_V_ERR_EMAIL_MISMATCH,
                        ];
                        debug!(indicator = %verify_indicator, x509_result = %x509_result.error(), "tls_verify_callback");

                        if !verify_indicator
                            && allowed_errors.contains(&x509_result.error().as_raw())
                        {
                            true
                        } else {
                            verify_indicator
                        }
                    },
                );
            }
            PostgresSslMode::VerifyFull => connector.set_verify(SslVerifyMode::PEER),
        };

        if let Some(rootcert) = certificates.rootcert.as_ref() {
            debug!(%rootcert, "loading CA bundle");
            connector.set_ca_file(rootcert).map_err(|e| {
                PsqlExporterError::PostgresTlsRootCertificate {
                    rootcert: (*rootcert).clone(),
                    cause: e,
                }
            })?;
        }

        if certificates.has_client_cert() {
            if let Some(cert) = certificates.cert.as_ref() {
                debug!(%cert, "loading client certificate");
                connector
                    .set_certificate_file(cert, SslFiletype::PEM)
                    .map_err(|e| PsqlExporterError::PostgresTlsClientCertificate {
                        filename: (*cert).clone(),
                        cause: e,
                    })?;
            }

            if let Some(key) = certificates.key.as_ref() {
                debug!(%key, "loading client private key");
                connector
                    .set_private_key_file(key, SslFiletype::PEM)
                    .map_err(|e| PsqlExporterError::PostgresTlsClientCertificate {
                        filename: (*key).clone(),
                        cause: e,
                    })?;
            }
        }

        let connector = MakeTlsConnector::new(connector.build());
        Ok(connector)
    }

    #[instrument("DbQuery", skip_all)]
    pub async fn query(
        &mut self,
        query: &str,
        query_timeout: Duration,
    ) -> Result<Vec<Row>, PsqlExporterError> {
        debug!(%query);

        let mut backoff_interval = self.default_backoff_interval;
        let mut sleeper = SleepHelper::from(self.shutdown_channel.clone());

        loop {
            // Set statement timeout
            let set_timeout_query = format!("set statement_timeout={};", query_timeout.as_millis());
            let result = self.client.query(set_timeout_query.as_str(), &[]).await;
            if let Err(e) = result {
                error!(error = %e);
                if e.code().is_none() {
                    self.reconnect().await?;
                } else {
                    return Err(PsqlExporterError::PostgresQuery {
                        query: set_timeout_query,
                        cause: e,
                    });
                }
            } else {
                // Execute actual query
                let result = self.client.query(query, &[]).await;
                if let Err(e) = result {
                    error!(error = %e);
                    if e.code().is_none() {
                        self.reconnect().await?;
                    } else {
                        return Err(PsqlExporterError::PostgresQuery {
                            query: query.to_string(),
                            cause: e,
                        });
                    }
                } else {
                    return Ok(result.unwrap());
                }
            }

            sleeper.sleep(backoff_interval).await?;
            backoff_interval += self.default_backoff_interval;
            if backoff_interval > self.max_backoff_interval {
                backoff_interval = self.max_backoff_interval;
            }
        }
    }

    #[instrument("DbReconnect", skip_all)]
    async fn reconnect(&mut self) -> Result<&Self, PsqlExporterError> {
        debug!("try to reconnect");
        let new_connection = PostgresConnection::new(
            self.db_connection_string.clone(),
            self.sslmode.clone(),
            self.certificates.clone(),
            self.default_backoff_interval,
            self.max_backoff_interval,
            self.shutdown_channel.clone(),
        )
        .await;

        match new_connection {
            Ok(conn) => {
                self.client = conn.client;
                self.connection_handler = conn.connection_handler;
                Ok(self)
            }
            Err(e) => {
                error!(error = %e);
                Err(e)
            }
        }
    }
}
