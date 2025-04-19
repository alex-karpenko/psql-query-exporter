use std::io;
use thiserror::Error;

#[derive(Error)]
pub enum PsqlExporterError {
    #[error("unable to load config file '{}': {}", .filename, .cause)]
    LoadConfigFile { filename: String, cause: io::Error },
    #[error("unable to parse config: {}", .cause)]
    ParseConfigFile {
        #[from]
        cause: serde_yaml_ng::Error,
    },
    #[error("unable to parse config: {0}")]
    InvalidConfigValue(String),
    #[error("some environment variable(s) not defined: {0}")]
    UndefinedEnvironmentVariables(String),
    #[error("unable to substitute environment variables: {0}")]
    EnvironmentVariableSubstitution(#[from] envsubst::Error),
    #[error("query failed '{}': {}", .query, .cause)]
    PostgresQuery {
        query: String,
        cause: tokio_postgres::Error,
    },
    #[error("failed to process query results: {0}")]
    PostgresQueryProcessing(#[from] tokio_postgres::error::Error),
    #[error("unable to create TLS connector: {}", .0)]
    PostgresTlsConnector(openssl::error::ErrorStack),
    #[error("unable to load CA certificate '{}': {}", .rootcert, .cause)]
    PostgresTlsRootCertificate {
        rootcert: String,
        cause: openssl::error::ErrorStack,
    },
    #[error("unable to load client certificate/key '{}': {}", .filename, .cause)]
    PostgresTlsClientCertificate {
        filename: String,
        cause: openssl::error::ErrorStack,
    },
    #[error("TLS client config error: {}", .0)]
    PostgresTlsClientConfig(String),
    #[error("shutdown signal has been received during operation")]
    ShutdownSignalReceived,
    #[error("unable to create metric '{}': {}", .metric, .cause)]
    CreateMetric {
        metric: String,
        cause: prometheus::Error,
    },
    #[error("unable to send task completion status: {}", .0)]
    MetricsBackStatusSend(#[from] tokio::sync::mpsc::error::SendError<usize>),
}

impl std::fmt::Debug for PsqlExporterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self)
    }
}
