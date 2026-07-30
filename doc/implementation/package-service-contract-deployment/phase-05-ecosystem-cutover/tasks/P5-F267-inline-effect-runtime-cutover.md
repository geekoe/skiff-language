# P5-F267 Inline effect Runtime cutover

## Dependencies

P5-F266.

## Required implementation

- Carry compiled inline effect plans through canonical test assembly/control
  into Runtime.
- Create one registry per case; preserve sequences, request-subset
  materialization, typed errors and streams.
- Report unused, exhausted, unexpected and nonmatching effects precisely.
- Delete the `skiff.test-doubles.json` loader, config reader, manifest schema
  and all compatibility paths.
- Reject old files explicitly as unsupported input until repository migration
  removes them; do not silently ignore them.
- Test config comes only from normal test service config/profile resolution.

## Acceptance

- Runtime/test-runner matrices cover unary, sequence, error, SSE, isolation,
  parallel cases and cross-Package effects.
- Account multi-request and Relay response tests pass using inline effects.
- Reverse search finds no production loader/schema reference to
  `skiff.test-doubles.json`.
- Workspace check, result and commit.
