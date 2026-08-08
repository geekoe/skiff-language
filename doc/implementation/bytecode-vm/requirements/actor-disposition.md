# Actor and Router lifecycle: requirement disposition

Phase 0 deliverable 6（`phase-0-baseline-live.md` §3.6）：把 [`../README.md`](../README.md) §5.8
（Actor and Router lifecycle）的已知问题转成**失败的 focused Rust 测试**，并为每条 requirement
记录 ledger disposition。失败是预期的——在改动 lifecycle production 代码之前证明问题存在。

## 失败测试位置与复现

全部失败测试位于 `router/src/actor/ownership.rs` 内嵌 `#[cfg(test)] mod tests`（`ownership.rs:516`
起，与 `owner_candidate.rs` 的内嵌测试风格一致；未新增文件、未改动 mod.rs、未改动任何 production 代码）。

复现命令（在 `/Users/geek/workspace/skiff-p0-router`，共享 CARGO_TARGET_DIR
`/Users/geek/workspace/.skiff-cargo-target`，命中增量缓存，编译约 4s）：

```bash
cargo test -p skiff-router --lib actor
```

期望输出摘要（已实测确认）：

```
running 6 tests
test actor::owner_candidate::tests::owner_selection_pins_ts_hash_modulo_candidates ... ok
test actor::ownership::tests::sweep_must_not_open_new_owner_while_idle_evict_is_unacked ... FAILED
test actor::ownership::tests::continuation_without_exact_build_proof_must_be_rejected ... FAILED
test task::actor_plan::tests::... (3 tests) ... ok
test result: FAILED. 4 passed; 2 failed; 0 ignored; 0 measured; 45 filtered out
```

两个失败断言：

1. `ownership.rs:650`（`sweep_must_not_open_new_owner_while_idle_evict_is_unacked`）：
   `lease expiry opened a new owner while the old incarnation may still exist (IdleEvict frames sent: 0, owner fence survived: false)`
   —— t=30s 的单个 sweep 里 `expire` 先于 idle-eviction 分支执行，fence 被静默清掉，
   `IdleEvict` 一帧都没发出（sent: 0），随后 `reserve` 对 runtime-b 成功。
2. `ownership.rs:710`（`continuation_without_exact_build_proof_must_be_rejected`）：
   `a continuation that cannot prove the exact build passed owner fence validation (renew result: Ok(ActorOwnerFence { ... lease_expires_at: 40000, ... }))`
   —— 无 build 标识的 stale continuation 通过 `renew` 校验并被续期到 40000。

既有测试不受影响：`owner_selection_pins_ts_hash_modulo_candidates` 与
`task::actor_plan::tests` 的 3 个测试全部通过（4 passed 即既有测试）。

## Requirement ledger（README §5.8 逐条）

| # | Requirement（README §5.8） | 状态 | 当前代码证据（文件:行） | 失败测试 | 目标阶段 |
| --- | --- | --- | --- | --- | --- |
| R1 | Actor 逻辑 identity 是 type + key/id，不是 service version 或 build；不同 Actor id 可同时 live 在不同 build | existing-needs-proof | `router/src/actor/types.rs:47-55`（`ActorLogicalKey` 无 build 字段）；`ownership.rs:75-88`（entry identity 固定于首次创建） | 无（本阶段不重复证明，Phase 7 验收 `phase-7-actor-router.md` §5: "id1/build A 与 id2/build B 并存"） | Phase 7 |
| R2 | live incarnation pin 一个 exact buildId；同 id 不同 build 的请求被拒绝、不升级、不刷新 idle/lease clock | missing（fence/incarnation 无 build 标识） | `types.rs:208-217`（`ActorOwnerFence` 无 build 字段）；`ownership.rs:263-271`（`commit` 丢弃 `token.route_authority` 的 `build_id`）；`ownership.rs:506-513`（`fence_identity_matches` 不比较 build） | `continuation_without_exact_build_proof_must_be_rejected`（`ownership.rs:671`） | Phase 3A（deployment owner cut，`phase-3a-deployment-owner.md` §3 逐条 pin exact buildId）→ 完成于 Phase 7（`phase-7-actor-router.md` §5: "same id/build A 已 live 时 build B claim 拒绝且 idle/lease deadline 不变化"） |
| R3 | 正常 idle eviction / disconnect / shutdown **实际销毁** incarnation 后，下一个 claimant 用其 exact build 创建；不保留 current/newest pointer | missing（eviction ack 顺序：lease expiry 先于 IdleEvict 完成/ack，等于未确认销毁就放行新 owner） | `lease.rs:131-192`（`sweep` 先 `registry.expire(now)` 再 idle-eviction 分支）；`ownership.rs:355-377`（`expire` 无条件清 fence 且清 `eviction_request_id`）；`types.rs:22-24`（`DEFAULT_OWNER_LEASE_TTL_MS` 与 `DEFAULT_IDLE_TTL_MS` 均 30s） | `sweep_must_not_open_new_owner_while_idle_evict_is_unacked`（`ownership.rs:596`） | Phase 7（`phase-7-actor-router.md` §5: "idle request 发出后 Router 在 ack 前不清 fence；owner lease expiry 不会形成双 owner"） |
| R4 | 第一版不跨 build 复用 live heap（即使 ABI 相同） | existing-needs-proof（router 域无跨 build heap 复用代码；heap 属 runtime 域） | router 侧无 heap；`ownership.rs:236-276`（commit 只接受 identity-exact facts） | 无（Phase 7 验收 "旧 heap/continuation 不可复活"，需 runtime 侧证据） | Phase 7 |
| R5 | **Add exact build to the owner fence and continuation validation. Router must not treat local owner-lease expiry as proof that Runtime state was destroyed.** | missing | build：`types.rs:208-217`、`ownership.rs:263-271`、`ownership.rs:506-513`（见 R2）；lease-expiry-as-proof：`lease.rs:135-141` + `ownership.rs:355-377` | 两个失败测试均覆盖（R2 的 continuation 校验 + R3 的 lease-expiry 不放行新 owner） | Phase 3A/7（README §7：exact-build fence 归 Phase 3A deployment owner cut 与 Phase 7 Actor/Router exact-build lifecycle；`phase-7-actor-router.md` §5: "stale build/fence/epoch/cancel continuation 拒绝且不重装 lease"、"Router lease expiry 不被当成 Runtime memory 已销毁的证明"） |
| R6 | Durable Actor task 冻结 exact build + activation snapshot，参与同一 claim/version 规则，不恢复已逐出的 in-memory Actor field | existing-needs-proof（task/admission 域，本任务未加测试） | `task/admission.rs`（actor-method admission lane） | 无（Phase 7 验收 "durable task 的 exact build、retry/lease recovery 和 snapshot facts 在 Router restart/Runtime replacement 后保持"） | Phase 7 |

状态口径：`missing` = README 要求的语义当前代码不存在（测试失败即证明）；`existing-needs-proof` =
语义存在或不在 router 域，但尚未有证据/测试证明，后续阶段补证据。

## 问题语义与机制（测试注释的完整版）

### 问题 A：idle/lease ordering（R3/R5 后半，Phase 7）

- 默认 `DEFAULT_OWNER_LEASE_TTL_MS` 与 `DEFAULT_IDLE_TTL_MS` 都是 30s（`types.rs:22-24`），
  lease 只在 method 活动时续期（`actor_sink.rs:654-662`）。
- `ActorLeaseExpiryScheduler::sweep`（`lease.rs:131-192`）在同一 tick 里先执行
  `registry.expire(now)`（`lease.rs:135`）再走 idle-eviction 分支（`lease.rs:143`）。
- t=30s 时两个时钟同时到期：`expire` 先清掉 owner fence（`ownership.rs:355-377`，连
  `eviction_request_id` 一起清），`IdleEvict` 从未发出（实测 sent: 0）；随后
  `reserve` 只检查 `lease_expires_at > now`（`ownership.rs:197`），fence 已不在 → 新 owner 可开。
- 等价地，即使 eviction 已 in-flight，`expire` 也会静默丢弃 eviction 状态，late ack 到达时
  `acknowledge_eviction` 返回 `EvictionMismatch`（`ownership.rs:437-458`），ack 丢失。
- 违反 README §5.8："A new owner cannot be opened while the old Runtime instance may still exist"。

### 问题 B：缺 exact-build fence（R2/R5 前半，Phase 3A/7）

- `ActorOwnerFence`（`types.rs:208-217`）只有 epoch / owner_runtime_id / owner_lease_id /
  lease_expires_at / abi / implementation / declaration_owner，**没有 build 字段**。
- claim token 携带 `ActorOwnerRouteAuthority { build_id }`（`types.rs:123-140`），但
  `commit` 构造 fence 时只取 `token.expected_epoch` / `owner_runtime_id`，build_id 被丢弃
  （`ownership.rs:263-271`）。
- `fence_identity_matches`（`ownership.rs:506-513`）逐字段比较，同样不含 build——因此
  "旧 build 的 continuation" 与当前 owner 在每个被比较字段上逐字节相同，`renew`/`release`
  校验通过（实测 renew 返回 `Ok` 并续期）。
- 违反 README §5.8："Add exact build to the owner fence and continuation validation"。

## 遗留问题

- R1/R4/R6 未加失败测试：R1/R4 属 runtime 侧（heap/incarnation）或本阶段无对应代码；R6 属
  `task/admission.rs` durable task 域，本任务写界只覆盖 router actor 模块。均在 Phase 7 补证据。
- 本任务只加测试与文档，未改动任何 production 代码；修复分派给 Phase 3A/7。
- 失败测试属于 Phase 0 预期产物（`phase-0-baseline-live.md` §3.6），在修复落地前保持 FAIL；
  修复时这些测试应转为通过，作为 Phase 7 验收的一部分。
