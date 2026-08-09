//! Typed MIR/CFG over File IR (Phase 2 WP4).
//!
//! MIR is the emitter's only semantic input (`emit_bytecode_artifact` in
//! `compiler/emission`). Construction is a post-pass over `FileIrUnit` plus
//! source-owned facts (expression types, callable effects, spans); the MIR
//! builder must not recover types/liveness/effects from File IR.
//!
//! File layout and API are owned by the WP4 worker (see
//! `doc/implementation/bytecode-vm/design/phase-2-compiler-emission.md` §2.4).
