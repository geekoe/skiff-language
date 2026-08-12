# TST0: Phase 1 Test Design Specification

> Status: completed

## 1. Scope

This specification covers the Phase 1 trusted scalar execution closure. It maps
each semantic dimension to a canonical scenario, fixture, observable assertion,
and owner. Unsupported dimensions are covered by a permanent fail-closed
negative companion.

## 2. Coverage matrix

| Dimension | Required Phase 1 scenarios | Fixture / injection | Observable assertion | Owner |
| --- | --- | --- | --- | --- |
| source/emission | literal, slot, branch, exact local call, return; unsupported construct rejected | repo fixture plus compiler admission tests | compile/emission succeeds or fails without fallback | compiler owner |
| artifact/admission | canonical artifact; size/index/target corruption | canonical store bytes | load/link/admission fails closed | runtime loader/linker owner |
| exact linking/image | exact deployment/entry; missing/mismatch; no first-package fallback | fixture store | wrong entry/owner rejected | runtime loader/linker owner |
| VM execution | operand/slot/local frame/result; fuel exhausted; deadline/internal-stop poll | scalar VCP and VM focused tests | exact scalar result or budget terminal | runtime VM owner |
| request boundary | deterministic success; VM failure projection; terminal exactly once | production request entry | payload `3.0`; non-unary mode rejected | request owner |
| lifecycle hygiene | request cleanup; no Pending/resource/child owner leak | scalar request only | no Pending, no stream/resource handle | request owner |
| structure | no semantic seal bypass, type equivalence, ambient artifact reread, alternate executor | reverse-search gate plus VCP composition | bypass count remains zero | Phase Gate owner |

## 3. Canonical evidence classes

Phase 1 Gate aggregates:

- focused/unit: VM scalar handlers, linker admission, compiler emission.
- producer-consumer contract: compiler handoff to store, store to loader.
- VCP: real fixture through production composition to `BoundaryResponse`.
- negative/lifecycle: corrupt bytecode, wrong entry, unsupported request mode.
- structural: no alternate executor or second authority.
- regression: existing `runtime/request` and `runtime/vm` tests remain selected.

## 4. Existing test disposition

- `runtime/request/tests/bytecode_request.rs::request_heap_scalar_returns_payload`
  stays as a focused helper for request projection.
- `runtime/vm/tests/vertical.rs` stays as focused VM/producer composition.
- The new `runtime/request/tests/bytecode_vm_phase_0_vcp.rs` is the canonical
  Phase 1 VCP carrier.
- No old artifact/image/request model harness is retained as canonical VCP.

## 5. Gate contract

The gate command must run the canonical selector, require at least one scenario,
reject skip, require the manifest, and reject candidate commit drift. The
gate wrapper is `scripts/run-bytecode-vm-phase-0-gate.mjs`.
