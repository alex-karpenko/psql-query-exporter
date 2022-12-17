use serde::Deserialize;
use std::{fmt::Display, time::Duration};
use tracing::{debug, error};

use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use tokio::task::JoinHandle;
use tokio_postgres::{Client, Error, Row};

const DB_CONNECTION_DEFAULT_BACKOFF_INTERVAL: Duration = Duration::from_secs(10);
const DB_CONNECTION_MAXIMUM_BACKOFF_INTERVAL: Duration = Duration::from_secs(300);

#[derive(Debug)]
pub struct PostgresConnection {
    db_connection_string: String,
    client: Client,
    connection_handler: JoinHandle<()>,
    ssl_verify: bool,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum PostgresSslMode {
    Disable,
    Allow,
    Prefer,
    Require,
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
            Self::Allow => "allow",
            Self::Require => "require",
            Self::Prefer => "prefer",
        };
        write!(f, "{s}")
    }
}

impl PostgresConnection {
    pub async fn new(db_connection_string: String, ssl_verify: bool) -> Result<Self, Error> {
        debug!("PostgresConnection::new: construct new postgres connection");
        let mut backoff_interval = DB_CONNECTION_DEFAULT_BACKOFF_INTERVAL;

        loop {
            let connector = TlsConnector::builder()
                .danger_accept_invalid_certs(!ssl_verify)
                .build();
            let connector = match connector {
                Ok(connector) => connector,
                Err(e) => panic!("error while creating TLS connector: {e}"),
            };
            let connector = MakeTlsConnector::new(connector);
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
                        ssl_verify,
                    });
                }
                Err(e) => {
                    error!("PostgresConnection::new: client error: {e}");
                }
            };

            tokio::time::sleep(backoff_interval).await;
            backoff_interval += DB_CONNECTION_DEFAULT_BACKOFF_INTERVAL;
            if backoff_interval > DB_CONNECTION_MAXIMUM_BACKOFF_INTERVAL {
                backoff_interval = DB_CONNECTION_MAXIMUM_BACKOFF_INTERVAL;
            }
        }
    }

    async fn reconnect(&mut self) -> Result<&Self, Error> {
        debug!("PostgresConnection::reconnect: try to reconnect");
        let new_connection =
            PostgresConnection::new(self.db_connection_string.clone(), self.ssl_verify).await;

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

    pub async fn query(&mut self, query: &String) -> Result<Vec<Row>, Error> {
        debug!("PostgresConnection::query: {query:?}");
        let mut backoff_interval = DB_CONNECTION_DEFAULT_BACKOFF_INTERVAL;

        loop {
            let result = self.client.query(query, &[]).await;
            if let Err(e) = result {
                error!("PostgresConnection::query: error: {e}");
                if e.code().is_none() {
                    debug!("PostgresConnection::query: try to reconnet after error");
                    self.reconnect().await?;
                } else {
                    return Err(e);
                }
            } else {
                return result;
            }

            tokio::time::sleep(backoff_interval).await;
            backoff_interval += DB_CONNECTION_DEFAULT_BACKOFF_INTERVAL;
            if backoff_interval > DB_CONNECTION_MAXIMUM_BACKOFF_INTERVAL {
                backoff_interval = DB_CONNECTION_MAXIMUM_BACKOFF_INTERVAL;
            }
        }
    }
}
