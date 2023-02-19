use crate::db::PostgresConnection;
use crate::scrape_config::{
    FieldType, ScrapeConfig, ScrapeConfigDatabase, ScrapeConfigQuery, ScrapeConfigValues,
};

use prometheus::core::{AtomicF64, AtomicI64, Collector, GenericGauge, GenericGaugeVec};
use prometheus::{
    opts, Encoder, Gauge, GaugeVec, IntGauge, IntGaugeVec, Opts, Registry, TextEncoder,
};
use tokio::sync::mpsc;
use tokio_postgres::Row;

use std::convert::Infallible;
use std::time::{Duration, SystemTime};

use tracing::{debug, error, warn};

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
    fn from(query_config: &ScrapeConfigQuery) -> Self {
        let mut metrics: Vec<MetricWithType> = vec![];

        match &query_config.values {
            ScrapeConfigValues::ValueFrom(values) => {
                let mut opts = opts!(
                    query_config.metric_name.clone(),
                    query_config.description.clone().unwrap()
                );

                if let Some(const_labels) = &query_config.const_labels {
                    opts = opts.const_labels(const_labels.clone());
                }

                let new_metric =
                    Self::helper_create_metric(&query_config.var_labels, &values.field_type, opts)
                        .unwrap_or_else(|e| {
                            panic!(
                                "error while creating metric {}: {}",
                                query_config.metric_name, e
                            )
                        });

                metrics.push(new_metric);
            }

            ScrapeConfigValues::ValuesWithLabels(values) => {
                for value in values {
                    let mut opts = opts!(
                        query_config.metric_name.clone(),
                        query_config.description.clone().unwrap()
                    );

                    if let Some(const_labels) = &query_config.const_labels {
                        let mut const_labels = const_labels.clone();
                        value.labels.iter().for_each(|(k, v)| {
                            const_labels.insert(k.to_string(), v.to_string());
                        });
                        opts = opts.const_labels(const_labels);
                    }
                    let new_metric = Self::helper_create_metric(
                        &query_config.var_labels,
                        &value.field_type,
                        opts,
                    )
                    .unwrap_or_else(|e| {
                        panic!(
                            "error while creating metric {}: {}",
                            query_config.metric_name, e
                        )
                    });

                    metrics.push(new_metric);
                }
            }

            ScrapeConfigValues::ValuesWithSuffixes(values) => {
                for value in values {
                    let metric_name = format!("{}_{}", query_config.metric_name, value.suffix);
                    let metric_desc = format!(
                        "{}: {}",
                        query_config.description.clone().unwrap(),
                        value.suffix
                    );
                    let mut opts = opts!(metric_name, metric_desc);

                    if let Some(const_labels) = &query_config.const_labels {
                        opts = opts.const_labels(const_labels.clone());
                    }
                    let new_metric = Self::helper_create_metric(
                        &query_config.var_labels,
                        &value.field_type,
                        opts,
                    )
                    .unwrap_or_else(|e| {
                        panic!(
                            "error while creating metric {}: {}",
                            query_config.metric_name, e
                        )
                    });

                    metrics.push(new_metric);
                }
            }
        };

        QueryMetrics {
            metrics,
            is_registered: false,
            last_updated: SystemTime::now() - query_config.metric_expiration_time,
            next_query_time: SystemTime::now(),
        }
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
                let metric = metric.to_collector();
                registry
                    .register(metric)
                    .unwrap_or_else(|e| panic!("error while registering metric: {e}"));
            }
            self.is_registered = true;
        };
    }

    fn unregister(&mut self, registry: &Registry) {
        if self.is_registered {
            for metric in self.metrics.iter() {
                let metric = metric.to_collector();
                registry
                    .unregister(metric)
                    .unwrap_or_else(|e| panic!("error while un-registering metric: {e}"));
            }
            self.is_registered = false;
        };
    }
}

pub async fn compose_reply() -> Result<impl warp::Reply, Infallible> {
    let registry = prometheus::default_registry();
    debug!("compose_reply: preparing metrics, registry={registry:?}");

    let mut buffer = vec![];
    let encoder = TextEncoder::new();
    let metric_families = registry.gather();
    encoder
        .encode(&metric_families, &mut buffer)
        .unwrap_or_else(|e| panic!("looks like a BUG: {e}"));

    Ok(String::from_utf8(buffer).unwrap_or_else(|e| panic!("looks like a BUG: {e}")))
}

pub async fn collecting_task(scrape_config: ScrapeConfig) {
    debug!("collecting_task: config={scrape_config:?}");
    let (tx, mut rx) = mpsc::channel(scrape_config.len());
    let sources = scrape_config.sources;
    for (_, source_db_instance) in sources {
        let databases = source_db_instance.databases;
        for database in databases {
            let tx = tx.clone();
            tokio::task::spawn(async move {
                collect_one_db_instance(database).await;
                tx.send(true).await.unwrap_or(());
            });
        }
    }

    while let Some(_msg) = rx.recv().await {
        warn!("collecting_task: one of the collect_one_db_instance threads has been completed");
    }
}

async fn collect_one_db_instance(database: ScrapeConfigDatabase) {
    debug!("collect_one_db_instance: start task for {database:?}");
    let mut db_connection = PostgresConnection::new(
        database.connection_string,
        database.sslmode.unwrap(),
        database.ssl_rootcert,
        database.ssl_cert,
        database.ssl_key,
        database.backoff_interval,
        database.max_backoff_interval,
    )
    .await
    .unwrap_or_else(|e| panic!("can't create db connection: {e}"));

    let registry = prometheus::default_registry();
    let mut query_metrics: Vec<QueryMetrics> = Vec::with_capacity(database.queries.len());
    database
        .queries
        .iter()
        .for_each(|q| query_metrics.push(QueryMetrics::from(q)));

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
                    query_metrics[index].register(registry);
                    match &query_item.values {
                        ScrapeConfigValues::ValueFrom(value) => {
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
                        ScrapeConfigValues::ValuesWithLabels(values) => {
                            for (value, metric) in values.iter().zip(&query_metrics[index].metrics)
                            {
                                update_metrics(
                                    &result,
                                    Some(&value.field),
                                    &query_item.var_labels,
                                    metric,
                                )
                            }
                        }
                        ScrapeConfigValues::ValuesWithSuffixes(values) => {
                            for (value, metric) in values.iter().zip(&query_metrics[index].metrics)
                            {
                                update_metrics(
                                    &result,
                                    Some(&value.field),
                                    &query_item.var_labels,
                                    metric,
                                )
                            }
                        }
                    }
                }
                Err(e) => {
                    if query_item.metric_expiration_time != Duration::ZERO {
                        let expiration_time =
                            query_metrics[index].last_updated + query_item.metric_expiration_time;
                        if SystemTime::now() > expiration_time {
                            debug!("deregister metrics as expired");
                            query_metrics[index].unregister(registry);
                        }
                    }
                    error!("{e}")
                }
            };
            query_metrics[index].next_query_time = SystemTime::now() + query_item.scrape_interval
        }

        let next_query_time = query_metrics
            .iter()
            .min_by(|x, y| x.next_query_time.cmp(&y.next_query_time))
            .map(|x| x.next_query_time)
            .expect("looks like a BUG");
        if next_query_time > SystemTime::now() {
            tokio::time::sleep(
                next_query_time
                    .duration_since(SystemTime::now())
                    .unwrap_or_else(|e| panic!("looks like a BUG: {e}")),
            )
            .await;
        }
    }
}

fn update_metrics(
    rows: &[Row],
    field: Option<&str>,
    var_labels: &Option<Vec<String>>,
    metric: &MetricWithType,
) {
    match metric {
        MetricWithType::SingleInt(metric) => {
            if let Some(field) = field {
                metric.set(rows[0].get(field))
            } else {
                metric.set(rows[0].get(0))
            }
        }
        MetricWithType::SingleFloat(metric) => {
            if let Some(field) = field {
                metric.set(rows[0].get(field))
            } else {
                metric.set(rows[0].get(0))
            }
        }
        MetricWithType::VectorInt(metric) => {
            for row in rows {
                let mut new_labels: Vec<String> = vec![];
                if let Some(labels) = var_labels {
                    for label in labels {
                        new_labels.push(row.get(label.as_str()));
                    }
                    let new_labels: Vec<&str> = new_labels.iter().map(AsRef::as_ref).collect();
                    let new_labels: &[&str] = new_labels.as_slice();
                    if let Some(field) = field {
                        metric.with_label_values(new_labels).set(row.get(field));
                    } else {
                        metric.with_label_values(new_labels).set(row.get(0));
                    }
                }
            }
        }
        MetricWithType::VectorFloat(metric) => {
            for row in rows {
                let mut new_labels: Vec<String> = vec![];
                if let Some(labels) = var_labels {
                    for label in labels {
                        new_labels.push(row.get(label.as_str()));
                    }
                    let new_labels: Vec<&str> = new_labels.iter().map(AsRef::as_ref).collect();
                    let new_labels: &[&str] = new_labels.as_slice();
                    if let Some(field) = field {
                        metric.with_label_values(new_labels).set(row.get(field));
                    } else {
                        metric.with_label_values(new_labels).set(row.get(0));
                    }
                }
            }
        }
    }
}
