use psql_query_exporter::{
    app_config::AppConfig, run_exporter, scrape_config::ScrapeConfig, utils::SignalHandler,
};
use std::{error::Error, net::SocketAddr};
use tracing::instrument;

#[tokio::main]
#[instrument("Main")]
async fn main() -> Result<(), Box<dyn Error>> {
    let app_config = AppConfig::new();
    let scrape_config = ScrapeConfig::from(&app_config.config)?;
    let addr = SocketAddr::from((app_config.listen_on, app_config.port));
    let signal_handler = SignalHandler::new()?;

    run_exporter(scrape_config, addr, signal_handler).await
}
