# T1 tooling cardinality repair

Status: implementation complete; awaiting integration

## Parent, authority, and DAG position

The direct parent is
[`a1-finding-wave-checkpoint.md`](./a1-finding-wave-checkpoint.md). Its
authority chain continues through the tail-call execution parent checkpoint to
the canonical architecture and runtime reference. T1 does not change tail-call
semantics; it closes the parent's genuine task-before tooling-test failure.

T1 is ready independently of R3 and must complete before the I3 combined probe.
The current candidate remains `PRE_ACCEPTANCE_BLOCKED`; completing T1 only
supplies its tooling-test input to that later merged checkpoint.

## Frozen input and preflight facts

- Baseline commit: `0583e9e097d8883644e5b6e1fb4d21055cbd05d6`
- Baseline tree: `93691e7fb6193ea15fb8fa4c3bc4cfab8d6f2637`
- Branch: `agent/t1-tooling-cardinality`
- Worktree: `/Users/geek/workspace/skiff-t1-tooling-cardinality`
- Integration owner: `/root/tco_integrator`

Zero-worktree Git-object inspection confirmed that the canonical
`COMMAND_EXECUTION_LEDGER` contains 11 unique owners: 9 `spawn` and 2
`execFile`. The focused test still expects 12 owners and 10 `spawn` owners.
The parent establishes that ancestor `37a18074f96f5e9710e0d9cfb8cc22aae4f8d32f`
removed the obsolete owner without updating those expectations.

## Ownership and completion contract

The implementation write is limited to
`scripts/tests/command-execution-policy.test.mjs`:

1. update the stale test title from twelve to eleven lifecycle owners;
2. update total and unique-owner expectations from 12 to 11;
3. update the `spawn` expectation from 10 to 9;
4. retain the two-`execFile`, no-`migration-pending`, and full production
   policy assertions.

This leaf must not modify the ledger, policy, scanner, discovery rules,
production scripts, runtime, or any tail-call implementation. It must not add
a fake owner. A ledger cardinality different from 11/9/2, a non-unique owner,
or any required change outside the owned test and this contract is a stop
condition.

## Risk and evidence

Risk is low: this is a test-only correction to match the canonical production
ledger. The focused proof and its unique T1 owner are:

```bash
/opt/homebrew/bin/node --test scripts/tests/command-execution-policy.test.mjs
```

The test must report a non-zero population and retain all ten test cases.
Formatting and a scoped diff/status review complete self-acceptance; T1 must
not run a selector or full gate. Any later change to the ledger, policy,
scanner, discovery rules, this test, Node toolchain, or source root invalidates
the evidence.

The implementation/result commit and tree, actual write set, focused result,
and reverse-search evidence must be handed directly to `/root/tco_integrator`
for serial integration and cleanup.

## Result

T1 changed only the stale title and the total, unique, and `spawn`
cardinalities in the owned policy test. The `execFile`,
`migration-pending`, and repository-wide `assertCommandExecutionPolicy(root)`
assertions remain unchanged. No ledger, scanner, discovery, policy,
production, or runtime file was modified.

| Evidence | Result |
| --- | --- |
| `/opt/homebrew/bin/node --test scripts/tests/command-execution-policy.test.mjs` | pass: 10 tests, 0 failures |
| Reverse search for the stale twelve-owner title and 12/10 expectations in the owned test | no matches |
| Reverse search for retained unique-owner, two-`execFile`, no-`migration-pending`, and production-policy assertions | all retained |
| `git diff --check` and scoped diff/status review | pass; only this contract and the owned test are changed |

T1 did not run a selector, full gate, live service, or stable instance.
