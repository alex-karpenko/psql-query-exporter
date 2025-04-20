use crate::config::{
    FieldType, ScrapeConfig, ScrapeConfigDatabase, ScrapeConfigQuery, ScrapeConfigValues,
};
use crate::db::{PostgresConnection, PostgresSslCertificates};
use crate::errors::PsqlExporterError;
use crate::utils::{ShutdownReceiver, SleepHelper};
use human_repr::HumanDuration;
use prometheus::core::{AtomicF64, AtomicI64, Collector, GenericGauge, GenericGaugeVec};
use prometheus::{
    opts, Encoder, Gauge, GaugeVec, IntGauge, IntGaugeVec, Opts, Registry, TextEncoder,
};
use std::collections::HashMap;
use std::time::{Duration, SystemTime};
use tokio::sync::mpsc;
use tokio_postgres::Row;
use tracing::{debug, error, info, instrument, warn};

#[derive(Debug)]
pub enum MetricWithType {
    SingleInt(GenericGauge<AtomicI64>),
    SingleFloat(GenericGauge<AtomicF64>),
    VectorInt(GenericGaugeVec<AtomicI64>),
    VectorFloat(GenericGaugeVec<AtomicF64>),
}

impl MetricWithType {
    fn to_collector(&self) -> Box<dyn Collector> {
        match self {
            MetricWithType::SingleInt(m) => Box::new(m.to_owned()),
            MetricWithType::SingleFloat(m) => Box::new(m.to_owned()),
            MetricWithType::VectorInt(m) => Box::new(m.to_owned()),
            MetricWithType::VectorFloat(m) => Box::new(m.to_owned()),
        }
    }
}

struct QueryMetrics {
    metrics: Vec<MetricWithType>,
    is_registered: bool,
    last_updated: SystemTime,
    next_query_time: SystemTime,
}

impl QueryMetrics {
    fn from(query_config: &ScrapeConfigQuery) -> Result<Self, PsqlExporterError> {
        let mut metrics: Vec<MetricWithType> = vec![];

        match &query_config.values {
            ScrapeConfigValues::ValueFrom { single: values } => {
                let mut opts = opts!(
                    query_config.metric_name.clone(),
                    query_config.description.clone().unwrap()
                );

                if let Some(const_labels) = &query_config.const_labels {
                    let const_labels: HashMap<String, String> =
                        const_labels.clone().into_iter().collect();
                    opts = opts.const_labels(const_labels);
                }

                let new_metric =
                    Self::helper_create_metric(&query_config.var_labels, &values.field_type, opts)
                        .map_err(|e| PsqlExporterError::CreateMetric {
                            metric: query_config.metric_name.clone(),
                            cause: e,
                        })?;

                metrics.push(new_metric);
            }

            ScrapeConfigValues::ValuesWithLabels {
                multi_labels: values,
            } => {
                for value in values {
                    let mut opts = opts!(
                        query_config.metric_name.clone(),
                        query_config.description.clone().unwrap()
                    );

                    let mut labels = HashMap::new();
                    value.labels.iter().for_each(|(k, v)| {
                        labels.insert(k.to_string(), v.to_string());
                    });

                    if let Some(const_labels) = &query_config.const_labels {
                        const_labels.iter().for_each(|(k, v)| {
                            labels.insert(k.to_string(), v.to_string());
                        });
                    }

                    opts = opts.const_labels(labels);
                    let new_metric = Self::helper_create_metric(
                        &query_config.var_labels,
                        &value.field_type,
                        opts,
                    )
                    .map_err(|e| PsqlExporterError::CreateMetric {
                        metric: query_config.metric_name.clone(),
                        cause: e,
                    })?;

                    metrics.push(new_metric);
                }
            }

            ScrapeConfigValues::ValuesWithSuffixes {
                multi_suffixes: values,
            } => {
                for value in values {
                    let metric_name = format!("{}_{}", query_config.metric_name, value.suffix);
                    let metric_desc = format!(
                        "{}: {}",
                        query_config.description.clone().unwrap(),
                        value.suffix
                    );
                    let mut opts = opts!(metric_name, metric_desc);

                    if let Some(const_labels) = &query_config.const_labels {
                        let const_labels: HashMap<String, String> =
                            const_labels.clone().into_iter().collect();
                        opts = opts.const_labels(const_labels);
                    }
                    let new_metric = Self::helper_create_metric(
                        &query_config.var_labels,
                        &value.field_type,
                        opts,
                    )
                    .map_err(|e| PsqlExporterError::CreateMetric {
                        metric: query_config.metric_name.clone(),
                        cause: e,
                    })?;

                    metrics.push(new_metric);
                }
            }
        };

        Ok(QueryMetrics {
            metrics,
            is_registered: false,
            last_updated: SystemTime::now() - query_config.metric_expiration_time,
            next_query_time: SystemTime::now(),
        })
    }

    fn helper_create_metric(
        var_labels: &Option<Vec<String>>,
        field_type: &FieldType,
        opts: Opts,
    ) -> Result<MetricWithType, prometheus::Error> {
        if let Some(var_labels) = var_labels {
            let new_labels: Vec<&str> = var_labels.iter().map(AsRef::as_ref).collect();
            match field_type {
                FieldType::Int => Ok(MetricWithType::VectorInt(IntGaugeVec::new(
                    opts,
                    &new_labels,
                )?)),
                FieldType::Float => Ok(MetricWithType::VectorFloat(GaugeVec::new(
                    opts,
                    &new_labels,
                )?)),
            }
        } else {
            match field_type {
                FieldType::Int => Ok(MetricWithType::SingleInt(IntGauge::with_opts(opts)?)),
                FieldType::Float => Ok(MetricWithType::SingleFloat(Gauge::with_opts(opts)?)),
            }
        }
    }

    fn register(&mut self, registry: &Registry) {
        self.last_updated = SystemTime::now();
        if !self.is_registered {
            for metric in self.metrics.iter() {
                let boxed_metric = metric.to_collector();
                if let Err(e) = registry.register(boxed_metric) {
                    error!(?metric, "error while registering metric: {e}");
                    return;
                } else {
                    debug!(?metric, "metric registered");
                }
            }
            self.is_registered = true;
        }
    }

    fn unregister(&mut self, registry: &Registry) {
        if self.is_registered {
            for metric in self.metrics.iter() {
                let boxed_metric = metric.to_collector();
                if let Err(e) = registry.unregister(boxed_metric) {
                    error!(?metric, "error while unregistering metric: {e}");
                    return;
                } else {
                    debug!(?metric, "metric unregistered");
                }
            }
            self.is_registered = false;
        }
    }
}

#[instrument("ComposeReply")]
pub async fn compose_reply(registry: Registry) -> String {
    debug!(?registry, "preparing metrics");

    let mut buffer = vec![];
    let encoder = TextEncoder::new();
    let metric_families = registry.gather();
    encoder
        .encode(&metric_families, &mut buffer)
        .unwrap_or_else(|e| panic!("looks like a BUG: {e}"));

    if buffer.is_empty() {
        warn!("no metrics found");
        return String::from("# no metrics found\n");
    }

    String::from_utf8(buffer).unwrap_or_else(|e| panic!("looks like a BUG: {e}"))
}

#[instrument("CollectorsTask", skip_all)]
pub async fn collectors_task(
    scrape_config: ScrapeConfig,
    registry: Registry,
    shutdown_channel: ShutdownReceiver,
) -> Result<(), PsqlExporterError> {
    debug!(config = ?scrape_config);

    if scrape_config.is_empty() {
        warn!("no sources configured, waiting for shutdown signal");
        let mut rx = shutdown_channel.clone();
        rx.changed()
            .await
            .map_err(|_| PsqlExporterError::ShutdownSignalReceived)?;
    } else {
        let mut handler_index: usize = 0;
        let (tx, mut rx) = mpsc::channel(scrape_config.len());
        let sources = scrape_config.sources;
        for (_, source_db_instance) in sources {
            let databases = source_db_instance.databases;
            for database in databases {
                let tx = tx.clone();
                let shut_rx = shutdown_channel.clone();
                let registry = registry.clone();
                tokio::spawn(async move {
                    let handler_result = collect_one_db_instance(database, registry, shut_rx).await;
                    let send_result = tx
                        .send(handler_index)
                        .await
                        .map_err(PsqlExporterError::MetricsBackStatusSend);

                    if let Err(result) = handler_result {
                        match result {
                            PsqlExporterError::ShutdownSignalReceived => {
                                debug!(task = %handler_index, "completed due to shutdown signal");
                                Ok(())
                            }
                            _ => {
                                error!(task = %handler_index, error=%result, "completed unexpectedly");
                                Err(result)
                            }
                        }
                    } else if let Err(result) = send_result {
                        Err(result)
                    } else {
                        handler_result
                    }
                });
                handler_index += 1;
            }
        }

        debug!(task = %handler_index, "handlers have been started");

        while let Some(task_index) = rx.recv().await {
            debug!(task = %task_index, "completed");
            handler_index -= 1;
            if handler_index == 0 {
                info!("all tasks have been stopped, exiting");
                return Ok(());
            }
        }
    }

    Ok(())
}

#[instrument("CollectSingleDbInstance", skip_all, fields(%database))]
async fn collect_one_db_instance(
    database: ScrapeConfigDatabase,
    registry: Registry,
    shutdown_channel: ShutdownReceiver,
) -> Result<(), PsqlExporterError> {
    if database.queries.is_empty() {
        warn!("no queries configured, exiting");
        return Ok(());
    }
    debug!("start task");

    let certificates =
        PostgresSslCertificates::from(database.sslrootcert, database.sslcert, database.sslkey)?;
    let mut db_connection = PostgresConnection::new(
        database.connection_string,
        database.sslmode.unwrap(),
        certificates,
        database.backoff_interval,
        database.max_backoff_interval,
        shutdown_channel.clone(),
    )
    .await?;

    let mut query_metrics: Vec<QueryMetrics> = Vec::with_capacity(database.queries.len());
    let mut sleeper = SleepHelper::from(shutdown_channel.clone());

    for q in database.queries.iter() {
        let metric = QueryMetrics::from(q)?;
        query_metrics.push(metric);
    }

    loop {
        for (query_item, index) in database.queries.iter().zip(0..query_metrics.len()) {
            if query_metrics[index].next_query_time > SystemTime::now() {
                continue;
            }

            let result = db_connection
                .query(&query_item.query, query_item.query_timeout)
                .await;

            match result {
                Ok(result) => {
                    query_metrics[index].register(&registry);
                    let update_result = match &query_item.values {
                        ScrapeConfigValues::ValueFrom { single: value } => {
                            if let Some(field) = &value.field {
                                update_metrics(
                                    &result,
                                    Some(field),
                                    &query_item.var_labels,
                                    &query_metrics[index].metrics[0],
                                )
                            } else {
                                update_metrics(
                                    &result,
                                    None,
                                    &query_item.var_labels,
                                    &query_metrics[index].metrics[0],
                                )
                            }
                        }
                        ScrapeConfigValues::ValuesWithLabels {
                            multi_labels: values,
                        } => {
                            let mut r = Ok(());
                            for (value, metric) in values.iter().zip(&query_metrics[index].metrics)
                            {
                                if let Err(e) = update_metrics(
                                    &result,
                                    Some(&value.field),
                                    &query_item.var_labels,
                                    metric,
                                ) {
                                    r = Err(e);
                                    break;
                                }
                            }
                            r
                        }
                        ScrapeConfigValues::ValuesWithSuffixes {
                            multi_suffixes: values,
                        } => {
                            let mut r = Ok(());
                            for (value, metric) in values.iter().zip(&query_metrics[index].metrics)
                            {
                                if let Err(e) = update_metrics(
                                    &result,
                                    Some(&value.field),
                                    &query_item.var_labels,
                                    metric,
                                ) {
                                    r = Err(e);
                                    break;
                                }
                            }
                            r
                        }
                    };
                    if let Err(e) = update_result {
                        error!("{e}")
                    }
                }
                Err(e) => {
                    if query_item.metric_expiration_time != Duration::ZERO {
                        let expiration_time =
                            query_metrics[index].last_updated + query_item.metric_expiration_time;
                        if SystemTime::now() > expiration_time {
                            debug!("deregister expired metrics");
                            query_metrics[index].unregister(&registry);
                        }
                    }
                    error!("{e}")
                }
            };
            query_metrics[index].next_query_time = SystemTime::now() + query_item.scrape_interval;
        }

        let next_query_time = query_metrics
            .iter()
            .min_by(|x, y| x.next_query_time.cmp(&y.next_query_time))
            .map(|x| x.next_query_time)
            .expect("looks like a BUG");

        let sleep_time;

        if next_query_time > SystemTime::now() {
            sleep_time = next_query_time
                .duration_since(SystemTime::now())
                .unwrap_or(Duration::from_micros(0));
        } else {
            sleep_time = Duration::from_micros(0);

            let slip_duration = SystemTime::now().duration_since(next_query_time).unwrap();
            let slip_duration = slip_duration.human_duration();
            warn!(sleep = %slip_duration, "overtimed query loop");
        }

        sleeper.sleep(sleep_time).await?;
    }
}

#[instrument("UpdateMetrics", skip_all)]
fn update_metrics(
    rows: &[Row],
    field: Option<&str>,
    var_labels: &Option<Vec<String>>,
    metric: &MetricWithType,
) -> Result<(), PsqlExporterError> {
    debug!(?rows, ?field, ?var_labels, ?metric);

    match metric {
        MetricWithType::SingleInt(metric) => {
            if let Some(field) = field {
                metric.set(rows[0].try_get(field)?);
            } else {
                metric.set(rows[0].try_get(0)?);
            }
        }
        MetricWithType::SingleFloat(metric) => {
            if let Some(field) = field {
                metric.set(rows[0].try_get(field)?)
            } else {
                metric.set(rows[0].try_get(0)?)
            }
        }
        MetricWithType::VectorInt(metric) => {
            for row in rows {
                let mut new_labels: Vec<String> = vec![];
                if let Some(labels) = var_labels {
                    for label in labels {
                        new_labels.push(row.try_get(label.as_str())?);
                    }
                    let new_labels: Vec<&str> = new_labels.iter().map(AsRef::as_ref).collect();
                    let new_labels: &[&str] = new_labels.as_slice();
                    if let Some(field) = field {
                        metric
                            .with_label_values(new_labels)
                            .set(row.try_get(field)?);
                    } else {
                        metric.with_label_values(new_labels).set(row.try_get(0)?);
                    }
                }
            }
        }
        MetricWithType::VectorFloat(metric) => {
            for row in rows {
                let mut new_labels: Vec<String> = vec![];
                if let Some(labels) = var_labels {
                    for label in labels {
                        new_labels.push(row.try_get(label.as_str())?);
                    }
                    let new_labels: Vec<&str> = new_labels.iter().map(AsRef::as_ref).collect();
                    let new_labels: &[&str] = new_labels.as_slice();
                    if let Some(field) = field {
                        metric
                            .with_label_values(new_labels)
                            .set(row.try_get(field)?);
                    } else {
                        metric.with_label_values(new_labels).set(row.try_get(0)?);
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::PostgresSslMode,
        test_utils::{
            create_test_connection_string, init_psql_server, init_tracing, TEST_DB_PASSWORD,
            TEST_DB_USER,
        },
    };
    use insta::assert_snapshot;
    use prometheus::Registry;
    use rstest::rstest;
    use std::env;
    use tokio::sync::watch;

    #[rstest]
    #[case("single", 2)]
    #[case("multi_labels", 2)]
    #[case("multi_suffixes", 2)]
    #[tokio::test]
    async fn test_collect_one_db_instance_single_basic(
        #[case] case_name: &str,
        #[case] number_of_updates: usize,
    ) {
        use insta::with_settings;
        use tokio::fs;

        init_tracing().await;
        let port = init_psql_server().await;

        let registry = Registry::new();
        let (tx, rx) = watch::channel(false);

        // Configure collectors task
        env::set_var("TEST_PG_PORT", format!("{port}"));
        env::set_var("TEST_PG_USER", TEST_DB_USER);
        env::set_var("TEST_PG_PASSWORD", TEST_DB_PASSWORD);

        let config =
            ScrapeConfig::from_file(&format!("tests/cases/{case_name}/config.yaml")).unwrap();
        let handler = tokio::spawn(collectors_task(config, registry.clone(), rx.clone()));
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Create side db connection to push updates
        let connection_string = create_test_connection_string(PostgresSslMode::Disable).await;
        let mut db = PostgresConnection::new(
            connection_string,
            PostgresSslMode::Disable,
            PostgresSslCertificates::from(None, None, None).unwrap(),
            Duration::from_secs(1),
            Duration::from_secs(5),
            rx,
        )
        .await
        .unwrap();

        // Get original data
        let metrics = compose_reply(registry.clone()).await;
        with_settings!(
            { description => format!("collector test case '{}', original data", case_name), omit_expression => true },
            { assert_snapshot!(format!("{case_name}_original"), metrics) }
        );

        // Update data
        for round in 1..=number_of_updates {
            // get queries string from file
            let queries = fs::read_to_string(format!("tests/cases/{case_name}/update_{round}.sql"))
                .await
                .unwrap();
            // Split it and filter out empty ones and comments
            let queries: Vec<&str> = queries
                .split('\n')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .filter(|s| !s.starts_with("--"))
                .collect();
            // Run queries one by one
            for query in queries {
                db.query(query, Duration::from_secs(1)).await.unwrap();
            }
            // Wait for more than 2s for the next scrape round
            tokio::time::sleep(Duration::from_secs(3)).await;
            let metrics = compose_reply(registry.clone()).await;
            with_settings!(
                { description => format!("collector test case '{case_name}', after update {round}"), omit_expression => true },
                { assert_snapshot!(format!("{case_name}_updated_{round}"), metrics) }
            );
        }

        // Wait for expiration
        tokio::time::sleep(Duration::from_secs(6)).await;
        let metrics = compose_reply(registry).await;
        with_settings!(
            { description => format!("collector test case '{}', expired", case_name), omit_expression => true },
            { assert_snapshot!(format!("{case_name}_expired"), metrics) }
        );

        tx.send(true).unwrap();
        handler.await.unwrap().unwrap();
    }
}
