#![allow(dead_code)]

// This target compiles the production owner module without pulling in unrelated
// File IR consumers that are assigned to the next strict-migration node.
#[path = "../../../src/assembly_execution/service_error_index.rs"]
mod service_error_index;
