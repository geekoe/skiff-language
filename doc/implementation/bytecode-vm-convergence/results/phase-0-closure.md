# Phase 0 Closure Result

> Status: accepted
>
> Accepted candidate: `b74b66589a9fe0307ed9a05014e33f3a19a1874a`
>
> Accepted tree: `4b720da227bc8c25838da7ce35d7eac6417295ed`
>
> Main integration merge: `4297bc75aedfd1058fe388d25d43ad996b1b9d5b`
>
> Evidence epoch: `skiff-bytecode-vm-phase-0-gate-v4`
>
> Acceptance verdict: `PASS`, no waivers

## 1. Baseline and contract

The recovery line started from clean `main` commit
`507779bedb009ec7789456995dd57df5e553739f`, tree
`e32a16a1fb88f72457cb7fa7547e9fca950270fa`. Its shared contract is
[`phase-0-supplemental-closure.md`](../tasks/phase-0-supplemental-closure.md), and its rolling execution record is
[`phase-0-recovery-execution-map.md`](../tasks/phase-0-recovery-execution-map.md), final revision 22.

The original [`phase-0.md`](./phase-0.md) receipt remains withdrawn historical evidence. It was not reused for this
acceptance.

## 2. Delivered production boundaries

The accepted recovery candidate establishes the following narrow facts for Phase 1:

- `e15bad88` materializes typed JSON `number`/`boolean`/`null` only from the exact pinned verified entry type and full
  trivial snapshot plan; malformed, mismatched, aggregate and unsupported plans fail closed, while RawHttp remains bytes;
- `dd1399bc` derives route/deployment/build/ingress/adapter facts from the admitted immutable image and does not reread the
  artifact root after admission or on cache hit;
- `2c9c2fa7` rejects non-unary bytecode HTTP before deployment load, target construction or VM dispatch;
- `5b305744` and `0da6e474` provide ordered, bounded, reentrant-safe production observations; reserve request identity
  before admission; reject duplicate ownership; preserve `Active -> Completing -> Cleanup` identity through terminal and
  cleanup; and keep observation callbacks outside supervisor locks;
- no new public execution authority was added. The proof carrier enters only through the host-owned
  `RuntimeHost::spawn_bytecode_request` composition seam frozen by
  [`dec0-vcp-production-seam.md`](../decisions/dec0-vcp-production-seam.md).

These changes do not declare aggregate lifecycle, writable paths, exceptions, Pending, streams, resources, child execution,
cross-owner heaps or broad verifier semantics accepted. They remain disabled or assigned to later phases.

## 3. Development and Proof receipts

The main write lanes and their independent receive checks were:

| Lane | Integrated commits | Independent disposition |
| --- | --- | --- |
| route/image pinning | `dd1399bc` | route receive review `PASS` |
| typed JSON materialization | `e15bad88` | typed JSON review `PASS` |
| HTTP mode containment | `2c9c2fa7` | containment review `PASS` |
| observation and lifecycle | `5b305744`, `0da6e474` | base candidates were rejected until the finalizing race proof; final review `PASS` |
| Gate integrity | `5c2c2113`, `df31adb8`, `8c6cd2db`, `b74b6658` | two false-green rounds rejected; final Gate review and invocation-contract review `PASS` |
| production proof support | `dab771ef`, `0c806307`, `3fff0a22` | support review `PASS` after visibility/path corrections |
| success proof | `0e104026`, `f655be1f`, `fb4ed909` | exact runtime proof and independent delta review `PASS` |
| negative proof | `fd935674`, `1c265d70`, `ba46ccda` | exact runtime proof and independent delta review `PASS` |

The execution map records the actual task IDs, worktrees, watchdog interventions, rejected candidates and takeover history.
The integrator performed only mechanical joins and receipt/status updates; Proof writers did not modify production to create a
PASS, and the final Acceptance Agent wrote nothing.

## 4. Canonical Gate and durable evidence

The freeze receipt supplied all identities as literals. The Gate did not select its own candidate or evidence directory:

```bash
SKIFF_BYTECODE_VM_PHASE0_CANDIDATE_COMMIT=b74b66589a9fe0307ed9a05014e33f3a19a1874a \
SKIFF_BYTECODE_VM_PHASE0_CANDIDATE_TREE=4b720da227bc8c25838da7ce35d7eac6417295ed \
SKIFF_BYTECODE_VM_PHASE0_EVIDENCE_DIR=/Users/geek/workspace/skiff-bcvm-p0-evidence-b74b6658 \
node scripts/verify.mjs --only bytecode-vm-phase-0-gate
```

The command ran once in detached worktree `/Users/geek/workspace/skiff-bcvm-p0-acceptance` and returned:

- `20/20` commands passed: twelve receipt-backed candidate probes and eight workloads;
- `33/33` declared tests passed: 26 Gate self-tests and seven exact Rust tests;
- zero failed, skipped, todo, cancelled or ignored tests;
- preflight, postflight, closure and fresh commit/tree/status snapshots all matched the frozen clean candidate;
- one production success, three fail-closed sub-scenarios and five focused regressions.

Durable evidence is at `/Users/geek/workspace/skiff-bcvm-p0-evidence-b74b6658`:

- manifest SHA-256: `96fc89ddfcd4149e8aa3a2bae23989d4f779ae0daf3d680d038c9a41553af22d`;
- bound command-environment SHA-256: `83dba90a472773ea9f8141bf2cd6471d738cfb41f0f1c9ff5ffbd44d57f36c97`;
- 20 receipts, 40 stdout/stderr streams and the directory-identity record are in the manifest hash closure;
- the evidence root contains 62 regular files and no symlink or non-regular evidence entry.

Key accepted source hashes:

- fixture: `46a4270198f4e33913592511b6560dc333eeb120afeb6d1f544512c99f62aab6`;
- success proof: `69e867f81a0c8ba76a0a8aa279a382cead960d2db8fdbdc9ad5226393f0f8935`;
- negative proof: `94aa58bab23160b1d76ef77bab7c9c97770e54480b2ded2fd523891590c39130`;
- Gate contract: `9b9a2488d6c6cc7b3ea48d12273889ead471c9415fcf0599df35f1148e2cdef3`;
- Gate runner: `37e312af6400287000d585a08999b1d4743b6892a543ca3234ae90f9ecc08616`;
- evidence checker: `06f370ed4870556129a6c50da557b4ebaff7faecee52a05914450b86be83e9de`.

## 5. Vertical closure proof

The success proof uses a real `.skiff` service fixture and the production compiler, canonical immutable publication/store,
host deployment load/admission/cache, exact gateway route and entry, production request entry and VM. Body `2` is materialized
as the pinned scalar argument; `main.run` calls a local helper returning `7`, takes the `== 7` branch, subtracts `4`, and the
wire response decodes to `3.0`.

After deterministic writer-channel closure the proof observes exactly five ordered, correlated production events:

1. deployment image selected;
2. exact route entry pinned;
3. first VM instruction successfully dispatched (`LoadSlot`);
4. successful terminal claimed;
5. request cleanup complete.

The three negative sub-scenarios change only one canonical input each: a valid JSON bytecode identity nibble, gateway identity,
or unary request mode. Each asserts the exact typed correlated error, zero observations and no second terminal after channel
closure. The harness constructs no registry route, linked/verified/executable image, request target, scheduler or fiber, and
does not mint observations or a verdict.

## 6. Phase 1 handoff

Phase 1 may now create MAP1 from the post-acceptance main receipt. Its first ready frontier is:

- Development: K0A compiler containment, K0B executable-closure containment and K0C request/route containment;
- Proof: T-C/T-R expected-red contracts, V1 production VCP evolution and G1 Gate/checker evolution.

The Phase 0 accepted seam is the host-owned production composition above. Phase 1 must still resolve, through its conditional
Design rules where the accepted handoff does not uniquely decide the target:

- the single immutable executable-image/entry authority after broad verifier/seal authority is removed;
- exact operation-entry semantics if operation routing remains in the accepted surface;
- hard fuel/deadline/internal-stop terminal ownership and exact executed-unit accounting;
- bounded non-verdict observations for frame/local-call/return and budget facts if the five Phase 0 events are insufficient.

Until those consumers close their own contracts, broad verifier semantics, alternate execution paths and all non-synchronous
capabilities remain disabled. Phase 1 cannot treat Phase 0's Gate PASS as acceptance of those capabilities.
