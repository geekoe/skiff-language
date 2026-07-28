# P5-F445H-I6E6 Actor current-scope consumer resume

状态：Ready。消费 E1 交付的 owned control，完成 Actor control、method、spawn 的调用点current scope
等待，并保留既有Actor segment与late-response fence。

## 直接父节点

- `P5-F445H-I6E1-shared-carrier-delivery-checkpoint-result.md`
- `P5-F445H-I6D-host-operation-current-scope-result.md`
- `P5-F445H-I6E-invocation-carrier-delivery-preflight-result.md`

## 固定输入

```text
E1 implementation commit  ba66719e03cbabde2e159b94761cc1a1c71b35d2
E1 implementation tree    0b1972158d710c4355274f7fb272be292dcc7927
integration base commit   e942efa99460ea2b9bf29f07d8dfe855c9715aff
integration base tree     46abc10c8fbdab6e70f2ea071539382dbf03a1be
```

## 行为要求

1. Actor get-or-create、replace、find、remove、method、spawn在操作开始时读取E1 current scope。
2. control/spawn的 `OutboundRequestLease` 与method的 `ActorMethodOutboundLease`继续拥有真实response
   waiter；scope lease与它们竞争，不新增第二个response registry。
3. current deadline、outer timeout、ancestor/internal stop胜出时drop waiter并保留late-response fence；
   response先本地提交时不被同刻signal覆盖。
4. method既有30s primitive仍只作为操作自身上限，与current deadline取更早者；不得把它提升为
   deployment/request默认。
5. internal stop不物化成用户错误；deadline/error继续由post-await checkpoint保留精确owner。
6. spawn只允许有效receipt唤醒`SpawnWorkerRegistry`；scope terminal后的late receipt不得唤醒。
7. 保持Actor segment切分、prepared operation、continuation resume与同步control helper语义；不新增
   public cancel/yield/lifecycle metadata。
8. 所有terminal路径释放lease/timer/waiter，drop fence继续拒绝late/duplicate response。

## 唯一写集

Production：

```text
runtime/eval/src/actor_dispatch.rs
runtime/eval/src/actor_dispatch/prepared_operation.rs
runtime/eval/src/spawn_ops.rs
runtime/host/src/eval_capability_adapter/actor.rs
runtime/host/src/capability_context/actor.rs
runtime/host/src/capability_context/actor_method_outbound.rs
```

Tests：

```text
runtime/eval/src/actor_dispatch/prepared_operation_tests.rs
runtime/eval/src/spawn_ops/canonical_tests.rs
runtime/host/src/eval_capability_adapter/actor.rs
runtime/host/src/capability_context/actor/tests.rs
runtime/host/src/capability_context/actor_method_outbound.rs
```

不得修改E1 `capabilities.rs`/shared API、native Actor dispatch、Router/wire、artifact/std或Cargo/lockfile。

## 测试

真实RED/GREEN覆盖control四入口、method、spawn；Ready/Pending、current/outer deadline、
ancestor/internal stop、30s primitive、normal竞争、late/duplicate、spawn wake fence、owner归零和
Eval→Host纵向receipt。

```text
cargo test -p skiff-runtime-eval f445h_i6_actor_scope -- --list
cargo test -p skiff-runtime-eval f445h_i6_actor_scope -- --nocapture
cargo test -p skiff-runtime-host f445h_i6_actor_scope -- --list
cargo test -p skiff-runtime-host f445h_i6_actor_scope -- --nocapture
cargo check -p skiff-runtime-eval -p skiff-runtime-host --locked
cargo fmt --check
git diff --check
```

两个selector listing均非零。

## 停止与禁止

若需要Actor/Router wire schema、新公开cancel/lifecycle metadata、修改E1共享接口，或现有post-await
checkpoint无法保留owner而必须新增跨层错误契约，提交 `TASK_SCOPE_EXPANDED` result并停止。禁止full
gate、stable/live/network/Mongo、merge/rebase/push。

## 完成

分开提交implementation/tests与
`P5-F445H-I6E6-actor-current-scope-resume-result.md`。result给出commit/tree、RED/GREEN、六类入口矩阵、
response/spawn fence与owner证据、实际写集，并标明 `I6_ACTOR_COMPLETE = YES/NO`。worktree保持clean。
