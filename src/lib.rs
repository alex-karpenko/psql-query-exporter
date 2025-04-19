pub mod cli;
pub mod config;
pub mod db;
pub mod errors;
pub mod metrics;
pub mod utils;

#[cfg(test)]
pub mod test_utils;

use axum::{response::Html, routing::get, Router};
use config::ScrapeConfig;
use metrics::collectors_task;
use prometheus::Registry;
use std::{error::Error, net::SocketAddr};
use tokio::net::TcpListener;
use tracing::{info, instrument};
use utils::{ShutdownReceiver, SignalHandler};

const HOME_PAGE_CONTENT: &str = include_str!("../assets/index.html");

#[instrument("RunExporter", skip_all)]
pub async fn run_exporter(
    scrape_config: ScrapeConfig,
    addr: SocketAddr,
    registry: Registry,
    mut signal_handler: SignalHandler,
) -> Result<(), Box<dyn Error>> {
    info!("starting metrics collector task");
    let metrics_collector_task = tokio::task::spawn(collectors_task(
        scrape_config,
        registry.clone(),
        signal_handler.get_rx_channel(),
    ));

    info!(address = %addr, "starting web server task");
    let http_server_task =
        tokio::task::spawn(web_server(addr, registry, signal_handler.get_rx_channel()));

    tokio::select! {
        biased;
        _ = signal_handler.shutdown_on_signal() => {},
        _ = metrics_collector_task => {info!("all collectors have been finished")},
        _ = http_server_task => {info!("web server has been finished")},
    }

    Ok(())
}

#[instrument("WebServer", skip_all, fields(addr))]
async fn web_server(
    addr: SocketAddr,
    registry: Registry,
    mut shutdown_rx: ShutdownReceiver,
) -> Result<(), std::io::Error> {
    let app = Router::new()
        .route("/", get(Html(HOME_PAGE_CONTENT)))
        .route("/health", get("healthy\n"))
        .route("/metrics", get(|| metrics::compose_reply(registry)));

    let listener = TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|_| panic!("unable to bind to address {:?}", addr));
    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        shutdown_rx.changed().await.unwrap();
    });

    server.await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{init_tracing, next_addr};
    use rstest::rstest;
    use std::time::Duration;
    use tokio::sync::watch;

    #[rstest]
    #[case("/", HOME_PAGE_CONTENT)]
    #[case("/health", "healthy\n")]
    #[case("/metrics", "# no metrics found\n")]
    #[tokio::test]
    async fn test_web_server_root(#[case] path: &str, #[case] expected: &str) {
        init_tracing().await;

        let addr = next_addr();
        let (tx, rx) = watch::channel(false);
        let registry = Registry::new();
        let server_task = tokio::spawn(async move { web_server(addr, registry, rx).await });
        tokio::time::sleep(Duration::from_millis(1)).await;

        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://{addr}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.text().await.unwrap(), expected);

        tx.send(true).unwrap();
        server_task.await.unwrap().unwrap();
    }
}
