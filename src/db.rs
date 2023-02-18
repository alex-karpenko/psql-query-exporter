use serde::Deserialize;
use std::{fmt::Display, time::Duration};
use tracing::{debug, error};

use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};
use postgres_openssl::MakeTlsConnector;
use tokio::task::JoinHandle;
use tokio_postgres::{Client, Error, Row};

#[derive(Debug)]
pub struct PostgresConnection {
    db_connection_string: String,
    client: Client,
    connection_handler: JoinHandle<()>,
    sslmode: PostgresSslMode,
    ssl_rootcert: Option<String>,
    default_backoff_interval: Duration,
    max_backoff_interval: Duration,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
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

impl PostgresSslMode {
    fn get_mode_str(&self) -> &str {
        match self {
            PostgresSslMode::Disable => "Disabled",
            PostgresSslMode::Prefer => "Prefer",
            PostgresSslMode::Require => "Require",
            PostgresSslMode::VerifyCa => "Verify-CA",
            PostgresSslMode::VerifyFull => "Verify-Full",
        }
    }
}

impl PostgresConnection {
    pub async fn new(
        db_connection_string: String,
        sslmode: PostgresSslMode,
        ssl_rootcert: Option<String>,
        default_backoff_interval: Duration,
        max_backoff_interval: Duration,
    ) -> Result<Self, Error> {
        debug!("PostgresConnection::new: construct new postgres connection");

        let mut backoff_interval = default_backoff_interval;

        loop {
            debug!("sslmode={}", sslmode.get_mode_str());
            let mut connector = SslConnector::builder(SslMethod::tls())
                .unwrap_or_else(|e| panic!("unable to create SSL connector: {}", e));

            match sslmode {
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
                            debug!(
                                "verify_callback, indicator={}, x509_result={}",
                                verify_indicator,
                                x509_result.error()
                            );

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

            if let Some(rootcert) = ssl_rootcert.as_ref() {
                debug!("loading CA bundle from {}", rootcert);
                connector
                    .set_ca_file(rootcert)
                    .unwrap_or_else(|e| panic!("unable to load PEM CA file {}: {}", rootcert, e));
            }

            let connector = MakeTlsConnector::new(connector.build());
            let connection = tokio_postgres::connect(&db_connection_string, connector).await;

            match connection {
                Ok((client, connection)) => {
                    let connection_handler = tokio::spawn(async move {
                        debug!("PostgresConnection::new: spawn new connection task");
                        if let Err(e) = connection.await {
                            error!("PostgresConnection: connection closed with error: {}", e);
                        }
                    });

                    return Ok(PostgresConnection {
                        client,
                        db_connection_string,
                        connection_handler,
                        sslmode,
                        ssl_rootcert,
                        default_backoff_interval,
                        max_backoff_interval,
                    });
                }
                Err(e) => {
                    error!("PostgresConnection::new: client error: {e}");
                }
            };

            tokio::time::sleep(backoff_interval).await;
            backoff_interval += default_backoff_interval;
            if backoff_interval > max_backoff_interval {
                backoff_interval = max_backoff_interval;
            }
        }
    }

    async fn reconnect(&mut self) -> Result<&Self, Error> {
        debug!("PostgresConnection::reconnect: try to reconnect");
        let new_connection = PostgresConnection::new(
            self.db_connection_string.clone(),
            self.sslmode.clone(),
            self.ssl_rootcert.clone(),
            self.default_backoff_interval,
            self.max_backoff_interval,
        )
        .await;

        match new_connection {
            Ok(conn) => {
                self.client = conn.client;
                self.connection_handler = conn.connection_handler;
                Ok(self)
            }
            Err(e) => {
                error!("PostgresConnection::reconnect: can't reconnect: {e}");
                Err(e)
            }
        }
    }

    pub async fn query(&mut self, query: &str, query_timeout: Duration) -> Result<Vec<Row>, Error> {
        debug!("PostgresConnection::query: {query:?}");
        let mut backoff_interval = self.default_backoff_interval;

        loop {
            // Set statement timeout
            let result = self
                .client
                .query(
                    format!("set statement_timeout={};", query_timeout.as_millis()).as_str(),
                    &[],
                )
                .await;
            if let Err(e) = result {
                error!("PostgresConnection::query: {e}");
                if e.code().is_none() {
                    debug!("PostgresConnection::query: try to reconnect after error");
                    self.reconnect().await?;
                } else {
                    return Err(e);
                }
            } else {
                // Execute actual query
                let result = self.client.query(query, &[]).await;
                if let Err(e) = result {
                    error!("PostgresConnection::query: {e}");
                    if e.code().is_none() {
                        debug!("PostgresConnection::query: try to reconnect after error");
                        self.reconnect().await?;
                    } else {
                        return Err(e);
                    }
                } else {
                    return result;
                }
            }

            tokio::time::sleep(backoff_interval).await;
            backoff_interval += self.default_backoff_interval;
            if backoff_interval > self.max_backoff_interval {
                backoff_interval = self.max_backoff_interval;
            }
        }
    }
}
