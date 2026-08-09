//! Phase 2 bounded const evaluator (WP5).
//!
//! Evaluates top-level const lowered expression DAGs at compile time and
//! produces `FrozenConstantGraph` values; the request-time executable
//! initializer body never enters bytecode images. Bounds and error model are
//! owned by the WP5 worker (see
//! `doc/implementation/bytecode-vm/design/phase-2-compiler-emission.md` §2.5).
