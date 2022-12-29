use crate::db::PostgresConnection;
use crate::scrape_config::{ScrapeConfig, ScrapeConfigDatabase, ScrapeConfigValues};

use prometheus::core::GenericGauge;
use prometheus::core::{AtomicF64, AtomicI64, GenericGaugeVec};
use prometheus::{Encoder, TextEncoder};
use tokio::sync::mpsc;
use tokio_postgres::Row;

use std::convert::Infallible;
use std::time::SystemTime;

use tracing::{debug, error, warn};

#[derive(Debug)]
pub enum MetricWithType {
    SingleInt(GenericGauge<AtomicI64>),
    SingleFloat(GenericGauge<AtomicF64>),
    VectorInt(GenericGaugeVec<AtomicI64>),
    VectorFloat(GenericGaugeVec<AtomicF64>),
}

pub async fn compose_reply() -> Result<impl warp::Reply, Infallible> {
    let registry = prometheus::default_registry();
    debug!("compose_reply: preparing metrics, registry={registry:?}");

    let mut buffer = vec![];
    let encoder = TextEncoder::new();
    let metric_families = registry.gather();
    encoder
        .encode(&metric_families, &mut buffer)
        .expect("looks like a BUG");

    Ok(String::from_utf8(buffer).expect("looks like a BUG"))
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

async fn collect_one_db_instance(mut database: ScrapeConfigDatabase) {
    debug!("collect_one_db_instance: start task for {database:?}");
    let mut db_connection = PostgresConnection::new(
        database.connection_string,
        database.ssl_verify.unwrap_or(true),
    )
    .await
    .expect("can't create db connection due to some fatal errors");

    loop {
        for item in database.queries.iter_mut() {
            if item.next_query_time > SystemTime::now() {
                continue;
            }

            let var_labels = &item.var_labels;
            let values = &item.values;
            let query = &item.query;

            let result = db_connection.query(query, item.query_timeout).await;

            match result {
                Ok(result) => match values {
                    ScrapeConfigValues::ValueFrom(value) => {
                        if let Some(field) = &value.field {
                            update_metrics(&result, Some(field), var_labels, &item.metric[0])
                        } else {
                            update_metrics(&result, None, var_labels, &item.metric[0])
                        }
                    }
                    ScrapeConfigValues::ValuesWithLabels(values) => {
                        for (value, metric) in values.iter().zip(&item.metric) {
                            update_metrics(&result, Some(&value.field), var_labels, metric)
                        }
                    }
                    ScrapeConfigValues::ValuesWithSuffixes(values) => {
                        for (value, metric) in values.iter().zip(&item.metric) {
                            update_metrics(&result, Some(&value.field), var_labels, metric)
                        }
                    }
                },
                Err(e) => error!("{e}"),
            };
            item.next_query_time = item.schedule_next_query_time();
        }

        let next_query_time = database
            .queries
            .iter()
            .min_by(|x, y| x.next_query_time.cmp(&y.next_query_time))
            .map(|x| x.next_query_time)
            .expect("looks like a BUG");
        if next_query_time > SystemTime::now() {
            tokio::time::sleep(
                next_query_time
                    .duration_since(SystemTime::now())
                    .expect("looks like a BUG"),
            )
            .await;
        }
    }
}

fn update_metrics(
    rows: &Vec<Row>,
    field: Option<&String>,
    var_labels: &Option<Vec<String>>,
    metric: &MetricWithType,
) {
    match metric {
        MetricWithType::SingleInt(metric) => {
            if let Some(field) = field {
                metric.set(rows[0].get(field.as_str()))
            } else {
                metric.set(rows[0].get(0))
            }
        }
        MetricWithType::SingleFloat(metric) => {
            if let Some(field) = field {
                metric.set(rows[0].get(field.as_str()))
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
                        metric
                            .with_label_values(new_labels)
                            .set(row.get(field.as_str()));
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
                        metric
                            .with_label_values(new_labels)
                            .set(row.get(field.as_str()));
                    } else {
                        metric.with_label_values(new_labels).set(row.get(0));
                    }
                }
            }
        }
    }
}
