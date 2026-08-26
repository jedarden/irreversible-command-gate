# ICG monitoring integration

The guard exposes a scrape-ready endpoint through the standalone monitor:

```sh
icg monitor --host 0.0.0.0 --port 8080
```

Endpoints:

- `GET /health/live` — process responsiveness and durable guard liveness.
- `GET /health/ready` — readiness for serving monitoring traffic.
- `GET /metrics` — Prometheus text exposition for health, deny-rate telemetry,
  rule-pack loading, denial-log availability, and denied operations by rule.

The monitor reads the same durable files used by hooks. Override them with
`ICG_HEALTH_PATH`, `ICG_TELEMETRY_PATH`, `ICG_DENIAL_LOG`, and `ICG_RULE_PACK`.
The default denial log path is `/var/cache/icg/denials.jsonl`. Denial payloads
are redacted by the hook unless `ICG_LOG_FULL_CONTENT=true` is explicitly set.

## Prometheus and Grafana

Merge [scrape.yml](prometheus/scrape.yml) into the existing Prometheus
configuration and load [alerts.yml](prometheus/alerts.yml) via `rule_files`.
The alerts cover target/process failures, high or anomalous deny rates,
rule-pack load errors, unavailable denial logs, and critical denials. Route the
`service: irreversible-command-gate` alerts through the normal Alertmanager
receivers; the repository does not assume a particular pager or chat backend.

Import [icg-overview.json](grafana/icg-overview.json) into Grafana with a
Prometheus data source. The dashboard shows guard state, uptime/crashes,
rolling deny-rate telemetry, rule-pack availability, and top denied rules.

## Operational telemetry semantics

`icg_baseline_*` is a descriptive rolling window over the retained individual
evaluations in `ICG_TELEMETRY_PATH` (up to the configured `window_size`, 1,000
by default). Each evaluation is one equally weighted sample: a denial is `1`,
and an allow, warning, or rewrite is `0`. Consequently,
`icg_baseline_mean`, `icg_baseline_deny_rate`, and
`icg_current_deny_rate` are the same value. `icg_baseline_stddev` is the
population standard deviation of those individual outcomes;
`icg_baseline_min` and `icg_baseline_max` are observed outcome indicators, not
per-release rates.

These metrics are useful for describing recent guard behavior and supporting
the high-deny-rate alert. They are not a confidence interval, a statistical
significance test, or release-comparison evidence. Automatic poison-pill
rollback uses the separate durable per-release baseline in
`session-state.json`; inspect it with `icg telemetry status` and follow the
[rollback runbook](../docs/runbooks/rollback.md).

## Denial log aggregation

[promtail/config.yml](promtail/config.yml) is a Loki/Promtail example. Mount
the directory containing `denials*.jsonl`, preserve the JSON fields as labels
shown there, and retain the raw line for incident review. The log store rotates
large files and keeps bounded history; ship both the active and rotated files.

Treat `command`, `content`, and `reason` as sensitive fields. Keep the default
redaction policy in place unless the log destination has an approved access
policy. Alerting should use metric labels (`pack_id`, `pattern_id`, and
`severity`) rather than indexing command text.

See [the deployment guide](../docs/monitoring-deployment-guide.md) and the
[incident runbook](../docs/runbooks/incident-response.md) for rollout and
response procedures.
