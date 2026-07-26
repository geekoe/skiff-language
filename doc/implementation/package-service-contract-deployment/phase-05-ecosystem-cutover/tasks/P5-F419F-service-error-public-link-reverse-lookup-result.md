# P5-F419F Service error public-link reverse lookup result

状态：**COMPLETE**。`public_artifact_identity_for_addr` 已改为从 Package schema public
entry 正向解析公开身份；compiler-current 同址 `Failure` / `main.Failure` 不再形成伪歧义，
F419E 最后一项 typed-throw fixture 已闭合。

## 1. Exact checkpoint 与 ancestry

| 项目 | commit | tree |
| --- | --- | --- |
| task worktree start | `6748b09242a108d9278883cf2b5319ba66bbfdab` | `19451cbdf18e4b68c2280a15c2255c96b381f447` |
| production/tests implementation checkpoint | `c7617af00b321f03d1680435bc3cedf851e16101` | `3d01afc15e39be2cbb6d6c44db586cc1271c054d` |

启动时和 implementation checkpoint 后均确认以下三个 commit 是 HEAD ancestor，逐项
`git merge-base --is-ancestor` 为 exit `0`：

```text
b611fe32b9814a0cef07550a1f3cfe7ef4f8333e
722c9070469aad98af5f38e515756d3897b0f4e4
7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d
```

implementation checkpoint 只修改：

```text
runtime/eval/src/assembly_execution/service_error_channel.rs
runtime/eval/src/assembly_execution/service_error_channel/tests.rs
```

## 2. Candidate 算法

旧算法先按 FileIR identity 与 type index 扫描全部
`PackageArtifact.implementation_links.types`，再要求同址 link 恰好一个，并把 link map key
当作 schema stable key。因此 public path `Failure` 与 implementation source path
`main.Failure` 指向同一声明时，会被错误地判为两个 public candidates。

新算法以公开 schema owner 为唯一事实源：

```text
PackageSchemaIndex entry
  -> entry.public_path
  -> implementation_links.types[public_path]
  -> exact loaded FileIR + type index
  -> entry stable key + PackageSchemaTypeId + exact record
```

具体保持以下失败关闭规则：

- schema index owner/identity、public nameability/path 与 index identity 必须 canonical；
- public path 必须有 exact implementation type link，且 FileIR identity、module、source hash 与
  type index 必须解析到已加载声明；
- artifact record ref 与 loaded record 必须保持 package owner、stable key、type id 一致；
- 没有 public schema entry 映射到执行地址时返回 `None`，implementation-only links 不进入候选；
- 恰好一个 public identity 匹配时返回该 identity；
- 两个不同 public identities 映射到同一地址时返回 `InvalidArtifact`，没有按地址任选或排序。

`ServiceErrorTypeIndex`、public export/import、opaque forwarding、correlation 与逐跳 stack
路径均未修改。

## 3. 正负回归

回归场景嵌入既有 test，测试总数保持任务给定的 `92 / 216`：

- public `Failure` 与同址 source link `main.Failure` 通过正常 assembly linker，并恢复
  `Failure` 的 exact package/key/type-id；
- 同址 links 仅为 implementation paths、schema index 无 public entry 时返回 `None`；
- 两个不同 public schema identities 同址时，正常 assembly admission 动态负例继续拒绝；
  同一 fixture 另构造可执行的函数级防御性 image，直接证明 reverse lookup 也拒绝；
- missing public path、把 public path 伪造成 `main.Failure`、missing public link、
  out-of-bounds exact coordinate 与 record key mismatch 均为 `InvalidArtifact`，不降级成
  private `InternalError`。

## 4. F419E 最后一项闭合

compiler-current error artifact 合法保留：

```text
Failure      -> public API path
main.Failure -> implementation source path
```

`source_inline_service_effect_sequence_typed_throw_is_caught_then_responds` 现已通过 exact
`1/1`，并随 `assembly_execution` 全集通过。F419E 的原 `7 passed / 1 blocked` 因而闭合为
`8/8`：真实 Package schema hydration、typed throw/catch 以及随后 ordered response 均已越过
原 production blocker。

## 5. 验证

所有 Cargo 命令使用共享 target：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

| 命令 | 结果 |
| --- | --- |
| `cargo test --locked -p skiff-runtime-eval service_error_channel -- --list` | PASS；`19 tests / 0 benchmarks` |
| `cargo test --locked -p skiff-runtime-eval service_error_channel` | PASS；`19/19` |
| typed-throw exact command | PASS；`1/1` |
| `cargo test --locked -p skiff-runtime-eval assembly_execution` | PASS；`92/92` |
| `cargo test --locked -p skiff-runtime-eval --lib` | PASS；`216/216` |
| `cargo check --locked -p skiff-runtime-eval` | PASS；只有既有 advisory warnings |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

## 6. 边界

没有修改 compiler、artifact model/identity、linker、shared test support、F419E fixture、
其它 production 或设计；没有派子 Agent，没有 merge/rebase/push，没有访问 stable/live、
instance 或 watch registry。
