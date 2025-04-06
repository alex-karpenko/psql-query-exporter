mod app_config;
mod db;
mod errors;
mod metrics;
mod scrape_config;
mod utils;

use app_config::AppConfig;
use axum::{response::Html, routing::get, Router};
use scrape_config::ScrapeConfig;
use std::error::Error;
use tokio::net::TcpListener;
use tracing::{info, instrument};
use utils::SignalHandler;

const HOME_PAGE_CONTENT: &str = include_str!("../assets/index.html");

#[tokio::main]
#[instrument]
async fn main() -> Result<(), Box<dyn Error>> {
    let app_config = AppConfig::new();
    let scrape_config = ScrapeConfig::from(&app_config.config)?;

    let app = Router::new()
        .route("/", get(Html(HOME_PAGE_CONTENT)))
        .route("/health", get("healthy\n"))
        .route("/metrics", get(metrics::compose_reply));

    let mut signal_handler = SignalHandler::new()?;
    let shutdown_channel_rx = signal_handler.get_rx_channel();

    let addr = std::net::SocketAddr::from((app_config.listen_on, app_config.port));
    let listener = TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|_| panic!("unable to bind to address {:?}", addr));
    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        signal_handler.shutdown_on_signal().await;
    });

    let metrics_collecting_task = tokio::task::spawn(metrics::collecting_task(
        scrape_config,
        shutdown_channel_rx.clone(),
    ));
    let http_server_task = tokio::task::spawn(async move { server.await });

    tokio::select! {
        _ = metrics_collecting_task => {info!("all collecting tasks have been finished")},
        _ = http_server_task => {info!("web server has been finished")},
    }

    Ok(())
}
