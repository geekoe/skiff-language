# Phase result template

每个阶段实施时复制本模板为`<phase-id>.md`。结果文档只记录证据和verdict，不重新定义阶段目标或语义。

```markdown
# <Phase>: result

Status: candidate-pass | complete | blocked
Evidence epoch: <id>

## Candidate

| Repository | Commit | Tree | Branch | Clean |
| --- | --- | --- | --- | --- |
| skiff | | | | |
| internals | | | | |
| skiff-packages | | | | |

Compiler/router/runtime absolute path and SHA:
Artifact root/profile/ports/Mongo:

## Requirement disposition

Closed:
Existing-needs-proof resolved:
Retirement-only resolved:
Open/blocking:

## Focused gates

| Command | Started/finished | Exit | Evidence hash/path | Result |
| --- | --- | --- | --- | --- |

## Phase-specific proof

List each acceptance clause and its production-shaped evidence.

## Isolated Live

Manifest path/hash:
Chat result:
Host check/full result:
Deployment engine/buildId map:
VM/GC/Pending/Actor counters required by this phase:

## Stable closure

Main merge commits:
Release pointer/buildIds:
Rebuilt binary SHA:
Stable chat result:
Stable host-tools result:

## Reverse-search and migration ledger

Allowed temporary legacy hits:
Deleted/replaced hits:
Fallback assertions:

## Performance/layout delta

Baseline/candidate:
Metrics/thresholds:

## Residual risks and next-phase owners

## Verdict

PASS | FAIL, with reasons.
```
