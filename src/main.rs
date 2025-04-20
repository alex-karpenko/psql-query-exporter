use prometheus::Registry;
use psql_query_exporter::{
    cli::CliParams, config::ScrapeConfig, run_exporter, utils::SignalHandler,
};
use std::{error::Error, net::SocketAddr};
use tracing::instrument;

#[tokio::main]
#[instrument("Main")]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = CliParams::new();
    let scrape_config = ScrapeConfig::from_file(&cli.config)?;
    let addr = SocketAddr::from((cli.listen_on, cli.port));
    let signal_handler = SignalHandler::new()?;
    let registry = Registry::new();

    run_exporter(scrape_config, addr, registry, signal_handler).await
}
