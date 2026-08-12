# AUD2: VM minimal executable core

> Status: completed

## 1. Admission

`Vm` and `VmFiber` only accept `VerifiedVmEntry`, which combines a verified
image, typed entry, and exact deployment pin (`runtime/vm/src/lib.rs`,
`runtime/vm/src/fiber.rs`). Entry arguments are checked before frame allocation
in `runtime/vm/src/admission.rs`.

## 2. Dispatch

The synchronous loop is in `runtime/vm/src/fiber.rs::run_segment_inner` and
`dispatch_one`. It:

- charges function entry and statement events;
- replenishes raw fuel through `VmBudget`;
- validates operand layout from the artifact opcode table;
- dispatches through a single match over `Opcode`;
- returns `VmControl` for child/adapter/stream/park boundaries.

## 3. Minimal scalar core

The following handlers form the Phase 1 scalar closure:

- `execute_const`
- `execute_load_slot`, `execute_store_slot`, `execute_move_slot`,
  `execute_take_slot`, `execute_pop`, `execute_dup`, `execute_drop`
- `execute_jump`, `execute_jump_if`, `execute_switch_tag`
- `execute_call_local`, `execute_tail_call_local`, `execute_return`
- `execute_binary_number`, `execute_number_comparison`, `execute_equality`
- budget/fuel and statement charge in `runtime/vm/src/budget.rs` and
  `runtime/vm/src/statement.rs`

## 4. Hidden dependencies

Scalar source often compiles through builtins or default package bindings. The
current VM already implements aggregate, throw, host effect, stream, callback,
service, actor, and interface opcodes. Phase 1 must either include those
dependencies in the closure or compile the MVP fixture without them; it must
not hide them behind a test-only fixture.

The lifecycle audit in `bytecode-vm-architecture-review.md` VM-01/VM-02 shows
that slot copy/drop and aggregate mutation are not currently driven by a single
linked lifecycle executor. That is a Phase 2 concern, but the Phase 1 support
matrix must prove those lanes fail closed.
