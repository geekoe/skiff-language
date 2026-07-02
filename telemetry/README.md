# Skiff Telemetry

`skiff-telemetry` runs as an independent Node process. Runtime telemetry reaches it over WebSocket at `/telemetry`; query APIs are served over HTTP on the same port.

Default local process settings live in `telemetry.yml`:

```yaml
telemetry:
  host: 127.0.0.1
  port: 4002
  path: /telemetry
```

Persistence is Mongo-backed, not file-backed. Use the shared project Mongo in deployment:

```yaml
mongo:
  url: mongodb://127.0.0.1:27017/?replicaSet=rs0&readPreference=primary&directConnection=true
  database: skiff
  ttlDays: 7
```

Run with `pnpm dev -- --config telemetry.yml`. `SKIFF_TELEMETRY_CONFIG` can also point to the file. Environment variables still override file values; `SKIFF_TELEMETRY_MONGO_URL` and `MONGO_URL` are accepted as Mongo URL fallbacks. For temporary local testing without Mongo, set `SKIFF_TELEMETRY_IN_MEMORY=true`; data is lost when the process exits.

Router config should point runtimes at the telemetry:

```yaml
telemetry:
  endpoint: ws://127.0.0.1:4002/telemetry
```
