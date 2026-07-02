# Skiff Language

Skiff is a backend language and runtime stack for describing services, APIs, data access, platform capabilities, tests, and deployment artifacts in one checked model.

The language is still pre-release. The implementation intentionally does not carry compatibility for old artifact or syntax formats unless the current tests and reference docs require it.

## Repository Layout

- `compiler/`, `syntax/`, `artifact-model/`, `artifact-identity/`: compiler and artifact identity crates.
- `runtime/`: Rust runtime crates and host process.
- `router/`: TypeScript service router and runtime control plane.
- `telemetry/`: TypeScript telemetry process.
- `scripts/`: CLI and local instance tooling.
- `std/`, `prelude/`: Skiff standard library sources.
- `test-runner/`: Skiff package and service test infrastructure.
- `doc/`: canonical language, runtime, and architecture documentation.

## Getting Started

Install Rust, Node.js, and pnpm, then install JavaScript dependencies for the packages you plan to work on.

Run the main test entry from the repository root:

```bash
pnpm test
```

Run all Rust workspace tests:

```bash
cargo test --workspace --no-fail-fast
```

Create an isolated local Skiff instance for runtime/router work:

```bash
node scripts/skiff.mjs instance init .skiff-instance/config.yml
node scripts/skiff.mjs instance up .skiff-instance/config.yml
node scripts/skiff.mjs instance status .skiff-instance/config.yml
```

Stop it with:

```bash
node scripts/skiff.mjs instance down .skiff-instance/config.yml
```

## Documentation

Start with:

- `doc/overview.md`
- `doc/reference/`
- `doc/architecture/`

Agent-oriented development instructions live in `AGENTS.md`.

## License

Skiff is licensed under the Apache License, Version 2.0. See `LICENSE`.
