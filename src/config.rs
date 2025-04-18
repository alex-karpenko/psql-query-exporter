use crate::{
    db::{PostgresConnectionString, PostgresSslMode},
    errors::PsqlExporterError,
};
use core::fmt::Display;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    env,
    fs::read_to_string,
    time::Duration,
};

const DEFAULT_SCRAPE_INTERVAL: Duration = Duration::from_secs(1800);
const DEFAULT_QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_METRIC_EXPIRATION_TIME: Duration = Duration::ZERO;
const DB_CONNECTION_DEFAULT_BACKOFF_INTERVAL: Duration = Duration::from_secs(10);
const DB_CONNECTION_MAXIMUM_BACKOFF_INTERVAL: Duration = Duration::from_secs(300);

#[derive(Deserialize, Serialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ScrapeConfig {
    #[serde(default)]
    defaults: ScrapeConfigDefaults,
    pub sources: BTreeMap<String, ScrapeConfigSource>,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(deny_unknown_fields, default)]
struct ScrapeConfigDefaults {
    #[serde(with = "humantime_serde")]
    scrape_interval: Duration,
    #[serde(with = "humantime_serde")]
    query_timeout: Duration,
    #[serde(with = "humantime_serde")]
    backoff_interval: Duration,
    #[serde(with = "humantime_serde")]
    max_backoff_interval: Duration,
    #[serde(with = "humantime_serde")]
    metric_expiration_time: Duration,
    metric_prefix: Option<String>,
    sslrootcert: Option<String>,
    sslcert: Option<String>,
    sslkey: Option<String>,
    sslmode: PostgresSslMode,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ScrapeConfigSource {
    host: String,
    #[serde(default = "ScrapeConfigSource::default_port")]
    port: u16,
    user: String,
    password: String,
    #[serde(default)]
    sslmode: Option<PostgresSslMode>,
    #[serde(with = "humantime_serde", default)]
    scrape_interval: Duration,
    #[serde(with = "humantime_serde", default)]
    query_timeout: Duration,
    #[serde(with = "humantime_serde", default)]
    backoff_interval: Duration,
    #[serde(with = "humantime_serde", default)]
    max_backoff_interval: Duration,
    #[serde(with = "humantime_serde", default)]
    metric_expiration_time: Duration,
    metric_prefix: Option<String>,
    sslrootcert: Option<String>,
    sslcert: Option<String>,
    sslkey: Option<String>,
    pub databases: Vec<ScrapeConfigDatabase>,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ScrapeConfigDatabase {
    pub dbname: String,
    #[serde(skip)]
    pub connection_string: PostgresConnectionString,
    #[serde(skip)]
    pub sslmode: Option<PostgresSslMode>,
    #[serde(with = "humantime_serde", default)]
    scrape_interval: Duration,
    #[serde(with = "humantime_serde", default)]
    query_timeout: Duration,
    #[serde(with = "humantime_serde", default)]
    pub backoff_interval: Duration,
    #[serde(with = "humantime_serde", default)]
    pub max_backoff_interval: Duration,
    #[serde(with = "humantime_serde", default)]
    metric_expiration_time: Duration,
    metric_prefix: Option<String>,
    #[serde(skip)]
    pub sslrootcert: Option<String>,
    pub sslcert: Option<String>,
    pub sslkey: Option<String>,
    pub queries: Vec<ScrapeConfigQuery>,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ScrapeConfigQuery {
    pub query: String,
    pub metric_name: String,
    pub description: Option<String>,
    metric_prefix: Option<String>,
    #[serde(with = "humantime_serde", default)]
    pub scrape_interval: Duration,
    #[serde(with = "humantime_serde", default)]
    pub query_timeout: Duration,
    #[serde(with = "humantime_serde", default)]
    pub metric_expiration_time: Duration,
    #[serde(default)]
    pub const_labels: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub var_labels: Option<Vec<String>>,
    #[serde(default)]
    pub values: ScrapeConfigValues,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(deny_unknown_fields, untagged)]
pub enum ScrapeConfigValues {
    #[serde(rename = "single")]
    ValueFrom { single: FieldWithType },
    #[serde(rename = "multi_labels")]
    ValuesWithLabels { multi_labels: Vec<FieldWithLabels> },
    #[serde(rename = "multi_suffixes")]
    ValuesWithSuffixes {
        multi_suffixes: Vec<FieldWithSuffix>,
    },
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct FieldWithType {
    pub field: Option<String>,
    #[serde(rename = "type", default)]
    pub field_type: FieldType,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct FieldWithLabels {
    pub field: String,
    #[serde(rename = "type", default)]
    pub field_type: FieldType,
    pub labels: BTreeMap<String, String>,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct FieldWithSuffix {
    pub field: String,
    #[serde(rename = "type", default)]
    pub field_type: FieldType,
    pub suffix: String,
}

#[derive(Deserialize, Serialize, Debug, Default)]
#[serde(deny_unknown_fields, rename_all = "lowercase")]
pub enum FieldType {
    #[default]
    Int,
    Float,
}

impl ScrapeConfig {
    pub fn from_file(path: &String) -> Result<ScrapeConfig, PsqlExporterError> {
        let config = read_to_string(path).map_err(|e| PsqlExporterError::LoadConfigFile {
            filename: path.clone(),
            cause: e,
        })?;
        let mut config: ScrapeConfig = serde_yaml_ng::from_str(&config)?;

        config.defaults.merge_env_vars()?;
        for (_name, instance) in config.sources.iter_mut() {
            instance.merge_env_vars()?;
            instance.propagate_defaults(&config.defaults);
        }

        Ok(config)
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }
}

impl Default for ScrapeConfigDefaults {
    fn default() -> Self {
        Self {
            scrape_interval: DEFAULT_SCRAPE_INTERVAL,
            query_timeout: DEFAULT_QUERY_TIMEOUT,
            backoff_interval: DB_CONNECTION_DEFAULT_BACKOFF_INTERVAL,
            max_backoff_interval: DB_CONNECTION_MAXIMUM_BACKOFF_INTERVAL,
            metric_expiration_time: DEFAULT_METRIC_EXPIRATION_TIME,
            metric_prefix: None,
            sslrootcert: None,
            sslcert: None,
            sslkey: None,
            sslmode: PostgresSslMode::default(),
        }
    }
}

impl ScrapeConfigDefaults {
    fn merge_env_vars(&mut self) -> Result<(), PsqlExporterError> {
        let envs = hashmap_from_envs();
        if let Some(rootcert) = self.sslrootcert.clone() {
            self.sslrootcert = Some(substitute_envs(&rootcert, &envs)?);
        }
        if let Some(cert) = self.sslcert.clone() {
            self.sslcert = Some(substitute_envs(&cert, &envs)?);
        }
        if let Some(key) = self.sslkey.clone() {
            self.sslkey = Some(substitute_envs(&key, &envs)?);
        }

        Ok(())
    }
}

impl ScrapeConfigSource {
    fn default_port() -> u16 {
        5432
    }

    fn propagate_defaults(&mut self, defaults: &ScrapeConfigDefaults) {
        let defaults = ScrapeConfigDefaults {
            scrape_interval: if self.scrape_interval == Duration::default() {
                self.scrape_interval = defaults.scrape_interval;
                defaults.scrape_interval
            } else {
                self.scrape_interval
            },
            query_timeout: if self.query_timeout == Duration::default() {
                self.query_timeout = defaults.query_timeout;
                defaults.query_timeout
            } else {
                self.query_timeout
            },
            backoff_interval: if self.backoff_interval == Duration::default() {
                self.backoff_interval = defaults.backoff_interval;
                defaults.backoff_interval
            } else {
                self.backoff_interval
            },
            max_backoff_interval: if self.max_backoff_interval == Duration::default() {
                self.max_backoff_interval = defaults.max_backoff_interval;
                defaults.max_backoff_interval
            } else {
                self.max_backoff_interval
            },
            metric_expiration_time: if self.metric_expiration_time == Duration::default() {
                self.metric_expiration_time = defaults.metric_expiration_time;
                defaults.metric_expiration_time
            } else {
                self.metric_expiration_time
            },
            metric_prefix: match self.metric_prefix {
                None => {
                    self.metric_prefix.clone_from(&defaults.metric_prefix);
                    defaults.metric_prefix.clone()
                }
                _ => self.metric_prefix.clone(),
            },
            sslrootcert: match self.sslrootcert {
                None => {
                    self.sslrootcert.clone_from(&defaults.sslrootcert);
                    defaults.sslrootcert.clone()
                }
                _ => self.sslrootcert.clone(),
            },
            sslcert: match self.sslcert {
                None => {
                    self.sslcert.clone_from(&defaults.sslcert);
                    defaults.sslcert.clone()
                }
                _ => self.sslcert.clone(),
            },
            sslkey: match self.sslkey {
                None => {
                    self.sslkey.clone_from(&defaults.sslkey);
                    defaults.sslkey.clone()
                }
                _ => self.sslkey.clone(),
            },
            sslmode: match self.sslmode {
                None => {
                    self.sslmode = Some(defaults.sslmode.clone());
                    defaults.sslmode.clone()
                }
                _ => self.sslmode.clone().unwrap(),
            },
        };

        self.databases.iter_mut().for_each(|db| {
            let conn_string = PostgresConnectionString {
                host: self.host.clone(),
                port: self.port,
                user: self.user.clone(),
                password: self.password.clone(),
                sslmode: self.sslmode.clone().unwrap(),
                dbname: db.dbname.clone(),
            };
            db.propagate_defaults(&defaults, conn_string);
        });
    }

    fn merge_env_vars(&mut self) -> Result<(), PsqlExporterError> {
        let envs = hashmap_from_envs();
        self.host = substitute_envs(&self.host, &envs)?;
        self.user = substitute_envs(&self.user, &envs)?;
        self.password = substitute_envs(&self.password, &envs)?;
        if let Some(rootcert) = self.sslrootcert.clone() {
            self.sslrootcert = Some(substitute_envs(&rootcert, &envs)?);
        }
        if let Some(cert) = self.sslcert.clone() {
            self.sslcert = Some(substitute_envs(&cert, &envs)?);
        }
        if let Some(key) = self.sslkey.clone() {
            self.sslkey = Some(substitute_envs(&key, &envs)?);
        }

        Ok(())
    }
}

impl ScrapeConfigDatabase {
    fn propagate_defaults(
        &mut self,
        defaults: &ScrapeConfigDefaults,
        connection_string: PostgresConnectionString,
    ) {
        self.connection_string = connection_string;
        let defaults = ScrapeConfigDefaults {
            scrape_interval: if self.scrape_interval == Duration::default() {
                self.scrape_interval = defaults.scrape_interval;
                defaults.scrape_interval
            } else {
                self.scrape_interval
            },
            query_timeout: if self.query_timeout == Duration::default() {
                self.query_timeout = defaults.query_timeout;
                defaults.query_timeout
            } else {
                self.query_timeout
            },
            backoff_interval: if self.backoff_interval == Duration::default() {
                self.backoff_interval = defaults.backoff_interval;
                defaults.backoff_interval
            } else {
                self.backoff_interval
            },
            max_backoff_interval: if self.max_backoff_interval == Duration::default() {
                self.max_backoff_interval = defaults.max_backoff_interval;
                defaults.max_backoff_interval
            } else {
                self.max_backoff_interval
            },
            metric_expiration_time: if self.metric_expiration_time == Duration::default() {
                self.metric_expiration_time = defaults.metric_expiration_time;
                defaults.metric_expiration_time
            } else {
                self.metric_expiration_time
            },
            metric_prefix: match self.metric_prefix {
                None => {
                    self.metric_prefix.clone_from(&defaults.metric_prefix);
                    defaults.metric_prefix.clone()
                }
                _ => self.metric_prefix.clone(),
            },
            sslrootcert: match self.sslrootcert {
                None => {
                    self.sslrootcert.clone_from(&defaults.sslrootcert);
                    defaults.sslrootcert.clone()
                }
                _ => self.sslrootcert.clone(),
            },
            sslcert: match self.sslcert {
                None => {
                    self.sslcert.clone_from(&defaults.sslcert);
                    defaults.sslcert.clone()
                }
                _ => self.sslcert.clone(),
            },
            sslkey: match self.sslkey {
                None => {
                    self.sslkey.clone_from(&defaults.sslkey);
                    defaults.sslkey.clone()
                }
                _ => self.sslkey.clone(),
            },
            sslmode: match self.sslmode {
                None => {
                    self.sslmode = Some(defaults.sslmode.clone());
                    defaults.sslmode.clone()
                }
                _ => self.sslmode.clone().unwrap(),
            },
        };

        self.queries.iter_mut().for_each(|q| {
            q.propagate_defaults(&defaults);
        });
    }
}

impl Display for ScrapeConfigDatabase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "host: {}, port: {}, user: {}, dbname: {}",
            self.connection_string.host,
            self.connection_string.port,
            self.connection_string.user,
            self.connection_string.dbname
        )
    }
}

impl ScrapeConfigQuery {
    fn propagate_defaults(&mut self, defaults: &ScrapeConfigDefaults) {
        self.scrape_interval = if self.scrape_interval == Duration::default() {
            defaults.scrape_interval
        } else {
            self.scrape_interval
        };
        self.query_timeout = if self.query_timeout == Duration::default() {
            defaults.query_timeout
        } else {
            self.query_timeout
        };
        self.metric_expiration_time = if self.metric_expiration_time == Duration::default() {
            defaults.metric_expiration_time
        } else {
            self.metric_expiration_time
        };
        self.metric_prefix = match self.metric_prefix {
            None => defaults.metric_prefix.clone(),
            _ => self.metric_prefix.clone(),
        };

        if let Some(prefix) = &self.metric_prefix {
            self.metric_name = format!("{}_{}", prefix, self.metric_name);
        }

        if self.description.is_none() {
            self.description = Some(self.metric_name.clone())
        }
    }
}

impl Default for ScrapeConfigValues {
    fn default() -> Self {
        Self::ValueFrom {
            single: FieldWithType {
                field: None,
                field_type: FieldType::Int,
            },
        }
    }
}

#[inline]
fn hashmap_from_envs() -> HashMap<String, String> {
    env::vars().collect()
}

fn substitute_envs(
    input: &str,
    envs: &HashMap<String, String>,
) -> Result<String, PsqlExporterError> {
    if envsubst::is_templated(input) {
        let result = envsubst::substitute(input, envs)?;
        // If there variable is still present - error
        if envsubst::is_templated(&result) {
            return Err(PsqlExporterError::UndefinedEnvironmentVariables(
                input.into(),
            ));
        }
        Ok(result)
    } else {
        Ok(input.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::with_settings;
    use rstest::rstest;

    #[test]
    fn test_substitute_envs() {
        env::set_var("POSTGRES_USER", "postgres");
        env::set_var("POSTGRES_PASSWORD", "password");
        env::set_var("POSTGRES_HOST", "localhost");
        env::set_var("POSTGRES_PORT", "5432");
        env::set_var("POSTGRES_DB", "psql_exporter");

        let envs = hashmap_from_envs();

        let text = "postgres://${POSTGRES_USER}:${POSTGRES_PASSWORD}@${POSTGRES_HOST}:${POSTGRES_PORT}/${POSTGRES_DB}";
        let result = substitute_envs(text, &envs).unwrap();
        assert_eq!(
            result,
            "postgres://postgres:password@localhost:5432/psql_exporter"
        );
    }

    #[test]
    fn test_substitute_envs_error() {
        let envs = hashmap_from_envs();

        let text = "postgres://${POSTGRES_USER2}:${POSTGRES_PASSWORD}@${POSTGRES_HOST}:${POSTGRES_PORT}/${POSTGRES_DB}";
        let result = substitute_envs(text, &envs);
        assert!(
            result.is_err(),
            "Expected error, but got result: {:?}",
            result
        );

        assert_eq!(
            result.unwrap_err().to_string(),
            format!("some environment variable(s) not defined: {text}")
        )
    }

    #[rstest]
    #[case("empty", 0)]
    #[case("defaults", 0)]
    #[case("envs", 1)]
    #[case("full", 3)]
    fn test_scrape_config_parsing(#[case] name: &str, #[case] len: usize) {
        use insta::assert_yaml_snapshot;

        env::set_var("TEST_PG_HOST", "host.from.env.com");
        env::set_var("TEST_PG_USER", "user_from_env");
        env::set_var("TEST_PG_PASSWORD", "password.from.env");
        env::set_var("TEST_PG_SSLROOTCERT", "/env/path/to/rootcert");
        env::set_var("TEST_PG_SSLCERT", "/env/path/to/cert");
        env::set_var("TEST_PG_SSLKEY", "/env/path/to/key");

        let config = ScrapeConfig::from_file(&format!("tests/configs/{name}.yaml")).unwrap();
        let snapshot_suffix = format!("scrape_config_parsing__{name}");
        with_settings!(
            { description => format!("config file: {name}"), omit_expression => true },
            { assert_yaml_snapshot!(snapshot_suffix, config) }
        );

        assert_eq!(config.len(), len);
        assert!(config.is_empty() == (len == 0));
    }

    #[test]
    fn test_scrape_config_database_display() {
        let db = ScrapeConfigDatabase {
            dbname: "testdb".to_string(),
            connection_string: PostgresConnectionString {
                host: "localhost".to_string(),
                port: 5432,
                user: "postgres".to_string(),
                password: "password".to_string(),
                sslmode: PostgresSslMode::Prefer,
                dbname: "testdb".to_string(),
            },
            sslmode: None,
            scrape_interval: Duration::default(),
            query_timeout: Duration::default(),
            backoff_interval: Duration::default(),
            max_backoff_interval: Duration::default(),
            metric_expiration_time: Duration::default(),
            metric_prefix: None,
            sslrootcert: None,
            sslcert: None,
            sslkey: None,
            queries: vec![],
        };

        assert_eq!(
            db.to_string(),
            "host: localhost, port: 5432, user: postgres, dbname: testdb"
        );
    }
}
