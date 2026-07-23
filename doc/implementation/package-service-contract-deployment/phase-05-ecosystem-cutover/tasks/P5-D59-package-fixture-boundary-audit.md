# P5-D59 Official package fixture boundary audit

- Authority: `doc/architecture/package-service-contract-deployment.md`, package artifact and
  canonical test-boundary requirements.
- Candidate: `/Users/geek/workspace/skiff-packages-phase-05-integration` at
  `ecb7485286fd4df6f2fed78022c75a2ad9c3cc36`.
- Scope: one assigned official package only: `aliyunoss`, `http-session`, `openai`, or `track`.
- Read-only task: execute or statically reduce the assigned package's canonical test path far enough
  to enumerate every boundary rejection, not merely the first surfaced error. Classify source,
  fixture, compiler/effect model, or task-environment ownership and list exact files and minimal
  positive/negative probes.
- Environment: use the Skiff integration checkout explicitly; do not use stable services, stable
  watch registry, or stable artifacts. No file edits, installs, commits, or full repository gate.
- Result: bounded finding report to the main Agent, including whether the package is already clean.

