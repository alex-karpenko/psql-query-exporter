# TODO

## Must have

- Documentation
- CI pipeline
- License
  
## Important improvements

- Implement TLS connection with custom CA and client's certificates.

## Not ungent changes

- Implement `query_timeout`.
- Make backoff intervals configurable.
- Implement config reload endpoint and functionality, possibly use config file change as a trigger.
- Implement metrics expiration after consecutive query failures.
- Optional JSON logs.
- Unit and integration tests.
