# psql-query-exporter

[Prometheus](https://prometheus.io/docs/introduction/overview/) exporter to produce metrics from [PostgreSQL](https://www.postgresql.org/) queries on periodic basis.

## Features

- Allows querying of multiple DB instances (hosts) and multiple databases within each instance independently.
- Flexible configuration of queries, sources (fields) of metric's values, labels, querying intervals, etc.
- Fast, lightweight and scalable.

## Usage

The easiest way to run exporter is to use [Docker image](#docker-image). If you use Kubernetes to run workload you can use [Helm chart](#helm-chart) to configure and deploy exporter. The third way to run exporter is to [build native Rust binary](#build-your-own-binary) using Cargo utility and run it.

Anyway, to run exporter we need a [configuration file](#configuration) with definition of the scraping targets: hosts, databases, queries, labels, etc.

### Docker image

Just use the following command to get usage help, the same as running it with `--help` command line option:

```bash
docker run --rm alexkarpenko/psql-query-exporter:latest
```

Typical output is:

```console
psql-query-exporter 0.1.0
Oleksii Karpenko <alexkarpenko@yahoo.com>
PostgreSQL Query Prometheus exporter

USAGE:
    psql-query-exporter [OPTIONS] --config <CONFIG>

OPTIONS:
    -c, --config <CONFIG>          Path to config file
    -d, --debug                    Enable extreme logging (debug)
    -h, --help                     Print help information
    -l  --listen-on <LISTEN_ON>    IP/hostname to listen on [default: 0.0.0.0]
    -p  --port <PORT>              Port to serve http on [default: 9090]
    -v, --verbose                  Enable additional logging (info)
    -V, --version                  Print version information
```

The only mandatory parameter is a path to configuration file. Detailed explanation of all possible configuration options is in the dedicated [Configuration](#configuration) section. Just for test purpose, there is an [example config](config.yaml) file to query PostgreSQL server at `localhost` for replication lag values. To use it:

```bash
docker run --rm --name psql-query-exporter -v $PWD/config.yaml:/config.yaml -e PG_USER=postgres -e PG_PASSWORD=postgres alexkarpenko/psql-query-exporter:latest --config /config.yaml -v
```

### Helm chart

Just create your own values file with overrides of the default values and your own config section and deploy Helm release to your K8s cluster:

```bash
helm install psql-query-exporter ./helm/psql-query-exporter -f my-values.yaml
```

For example your values can be like below. Don't forget to create secret `psql-query-exporter` with two keys `PG_USER` and `PG_PASSWORD` with username and password to access DB.

```yaml
# info or debug, anything else - warning
logLevel: info

secrets:
  - psql-query-exporter

config:
  defaults:
    scrape_interval: 30s
    query_timeout: 5s
    backoff_interval: 10s
    max_backoff_interval: 300s

  sources:
    postgres:
      host: psql-server.postgres.svc.cluster.local
      user: ${PG_USER}
      password: ${PG_PASSWORD}
      sslmode: require
      metric_prefix: postgres_state

      databases:
        - dbname: postgres
          queries:
            - metric_name: replication_lag
              description: Storage size and state of replication slots
              query: |
                select slot_name, slot_type, active::text, 
                (case when not pg_is_in_recovery() then pg_current_wal_lsn() - restart_lsn end)::float as lag_bytes
                from pg_replication_slots;
              values:
                single:
                  field: lag_bytes
                  type: float
              var_labels:
                - slot_name
                - slot_type
                - active

```

### Build your own binary

Since exported is written in Rust, you can use standard Rust tools to build binary for any platform you need. Of course, you have to have [Rust](https://rust-lang.org) tool-chain installed.

```bash
cargo build --release
```

And run it:

```bash
target/release/psql-query-exporter --config ./config.yaml -v
```

## Configuration

Configuration file has two sections: optional `defaults` and mandatory `sources`.

### Defaults

`defaults` intended to describe config-wide defaults, all values from this section will be applied to all sources/databases/queries from the `sources` section if the particular value isn't overridden in the corresponding section (if applicable).

Here is default content of the section, so if you don't specify any value these defaults will be applied:

```yaml
defaults:
  scrape_interval: 30m  # interval to run each query,
                        # may be overridden by source/db/query config

  query_timeout: 10s    # timeout to wait for result of each query, 
                        # may be overridden by source/db/query config

  metric_prefix: ""     # prefix for metric name, 
                        # may be overridden by source/db/query config

  sslmode: prefer       # SSL mode to connect to the DB, optional,
                        # possible values are: disable, prefer, require, verify-ca and verify-full
                        # may be overridden by source/ config

  sslrootcert: ""       # path to additional root (CA) certificates file
                        # should be in PEM format and may contain more than one certificate
                        # may be overridden by source config

  sslcert: ""           # path to client certificates and key files
  sslkey: ""            # should be in PEM format
                        # may be overridden by source config

  metric_expiration_time: 0s # if all query attempts during this time were failed,
                             # then metric should be excluded from the output 
                             # until first successful query execution

  backoff_interval: 10s # default interval between failed connection attempts
  max_backoff_interval: 300s # every time after failed connection to the DB
                             # interval between connection attempts increases 
                             # by value of backoff_interval, but no more than value
                             # of the max_backoff_interval


```

### Sources

Second (mandatory) section is `sources`, it describes:

- which queries should be run,
- on which DBs,
- and how to extract and present results as metrics.

So `sources` section is a dictionary, each key is mnemonic name of the source. Each source is a database instance (DB host) with connection parameters (hostname, username, password, etc.) and list of `databases` inside that instance. Each `database` in the list contains list of `queries` to run against that DB with attributes that describe how to interpret query results and create metrics from them.

#### Sources definition

In the `host`, `user`, `password`, `sslrootcert`, `sslcert` and `sslkey` values environment variables can be used to set whole value of the field or part of it, by replacing `${NAME}` with value of the `NAME` environment variable. For example:

```yaml
  host: db.${ENV_NAME}.example.com
  user: ${PG_USER}
  password: ${PG_PASSWORD}
```

#### Some important remarks about queries

- Query can by any arbitrary SQL query that returns at least one numeric value (int of float column). This value is used as a gauge metric's value.
- If query returns more than one values (columns), than either first column is used for metric's value (default) or you should explicitly specify metric's source in `values.single` section.
- Query can return more than one valuable columns. In such case you should explicitly describe how to interpret each value and associate each one with either some additional label(s) (`values.multi_labels`) or create separate metrics for each value (`values.multi_suffixes`) by adding suffix to the metric's name.
- If value of metric has float (not default integer) type you should explicitly specify it's type.
- You can add arbitrary label/value pair(s) to the metric (`const_labels`).
- You can add variable labels (`var_labels`) to the metric using query result as a source for values of the labels. In such case query should return non-numeric values (columns) with string type (char, varchar, text).
- It's your responsibility to write query that returns value(s) with correct type of the fields: int/float for the metric's values and char/varchar/text for the labels. Exporter doesn't validate query statement or guess result's types, it just expect correct column's type.
- `single`, `multi_labels` and `multi_suffixes` sub-sections in the `values` section of the query definition are mutually-exclusive.

#### Detailed configuration with explanation

Below is a detailed explanation of all possible configuration parameters. Default values for optional parameters are specified.

Values of `defaults` (from the previous section) will be propagated to all underneath sections, level by level. So if you specify some value in the `defaults` section, than it will be used in each source, database and query if you don't override it at any level. If you change value of some parameter, it will be propagated in all underneath sub-sections of the section where it was changed.

```yaml
sources:
  source_name_1: # name of the source, just for convenience
    host: ""  # hostname of the DB instance, mandatory,
              # environment variable can be used here
    port: 5432  # port number of the DB, default is 5432
    user: ""  # username to login to the DB, mandatory,
              # environment variable can be used here
    password: ""  # password to login to the DB, mandatory,
                  # environment variable can be used here
    sslmode: prefer   # SSL mode to connect to the DB, optional,
                      # possible values are: disable, prefer, require, verify-ca and verify-full
    sslrootcert: ""   # path to additional root (CA) certificates file
                      # should be in PEM format and may contain more than one certificate
    sslcert: ""       # path to client certificates and key files
    sslkey: ""        # should be in PEM format
                      # may be overridden by source config
    scrape_interval: 30m  # scrape interval for all DBs/queries of the source, optional,
                          # overrides value from the default section,
                          # can be overridden in DB/query section
    query_timeout: 10s  # value of the query timeout for all DBs/queries of the source, optional,
                        # overrides value from the default section,
                        # can be overridden in DB/query section
    metric_expiration_time: 0s  # if all query attempts during this time were failed,
                                # then metric should be excluded from the output 
                                # until first successful query execution
    backoff_interval: 10s # default interval between failed connection attempts
    max_backoff_interval: 300s # every time after failed connection to the DB
                              # interval between connection attempts increases 
                              # by value of backoff_interval, but no more than value
                              # of the max_backoff_interval
    metric_prefix: "" # will be added to names of the all metrics for these DBs/queries, optional,
                      # overrides value from the default section,
                      # can be overridden in DB/query section

    databases:   # list of the databases inside the instance, mandatory
      - name: ""  # DB name, mandatory
        scrape_interval: 30m  # the same as above, applied to all queries of the DB, optional
        query_timeout: 10s    # the same as above, applied to all queries of the DB, optional
        metric_expiration_time: 0s  # if all query attempts during this time were failed,
                                    # then metric should be excluded from the output 
                                    # until first successful query execution
        backoff_interval: 10s # default interval between failed connection attempts
        max_backoff_interval: 300s # every time after failed connection to the DB
                                  # interval between connection attempts increases 
                                  # by value of backoff_interval, but no more than value
                                  # of the max_backoff_interval
        metric_prefix: ""     # the same as above, applied to all queries of the DB, optional

        queries:  # list of queries to run against this particular instance/db, mandatory
          - query: "" # query string, mandatory
            description: "" # Metric's description, it will be presented in HELP part of the metric's output
                            # If metric has multi_suffixes (see below) than suffix will be added to the description after semicolon
                            # Default is metric's name
            metric_name: "" # name that will be joined with the metric_prefix and underscore, mandatory
                            # if metric_prefix is empty, metric_name is used to form final name of the metric
            scrape_interval: 30m  # the same as above, applied to this query, optional
            query_timeout: 10s    # the same as above, applied to this query, optional
            metric_expiration_time: 0s  # if all query attempts during this time were failed,
                                        # then metric should be excluded from the output 
                                        # until first successful query execution
            metric_prefix: ""     # the same as above, applied to this query, optional

            # All values below are just for example, it's not default values.
            const_labels:           # all key/value pairs of this section will be added to the metric definition(s) of the query, optional
              label1: label_value1  # if metric_prefix="some_prefix" and metric_name="metric" than result metric will look like
              label2: label_value2  # some_prefix_metric{label1="label_value1",label2="label_value2"}

            var_labels: # if query result has text column(s), they can be used as label values
              - label1  # in such case you should specify column names here as label names
              - label2  # values from the columns will be used as label values

            values: # if you need to explicitly specify metric's source or query returns multi-value result,
                    # you should use this section to describe how to grab value(s)
              single: # use single field as a source
                field: field1
                type: int # int (default) or float, optional
              multi_labels: # use several fields and differentiate and create single metric with different additional labels
                - field: field2
                  type: int # int (default) or float, optional
                  labels:
                    label1: label_value1
                    label2: label_value2
                - field: field3
                  type: int # int (default) or float, optional
                  labels:
                    label1: label_value1
                    label3: label_value3
              multi_suffixes: # create separate metric for each value by adding suffix to the metric name
                - field: field4
                  type: int # int (default) or float, optional
                  suffix: suffix1
                - field: field5
                  type: int # int (default) or float, optional
                  suffix: suffix2

          - query: "" # next query from the same db
            .
            .
            .
          - query: ...

      - name: "" # next db at the same instance
        .
        .
        .
source_name_2:
  .
  .
  .
source_name_3:
  .
  .
  .
```

#### Threads and timings

Each database (not DB instance, but each item in the `sources.databases` list) uses its own thread to run querying loop. In other words, one list of queries uses its own thread to process all queries. Just for example, if you have three DB instances (source) in the config with five DB name in each instance (five items in the databases list) than 15 thread will be run to serve querying process.

Each thread is lightweight, and spend almost all time sleeping and waiting for time to run next query in list. So if you need to run heavy queries with long running-time be cautious and pay some attention to such parameters as `query_timeout` and `scrape_interval`, because each query in the list within each database entry will be running one-by-one with respect to scrape interval of each query.

For example, if you set scrape interval to 10s and query timeout to 5s and each of 2 queries in the list need 5s to return result, than all other queries within the same database will be postponed until end of that two.
