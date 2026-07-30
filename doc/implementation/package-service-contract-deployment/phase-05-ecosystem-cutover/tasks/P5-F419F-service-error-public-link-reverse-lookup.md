# P5-F419F Service error public-link reverse lookup

状态：Ready。

## 直接父节点

- `P5-F419E-suspension-runtime-current-fixture-repair-result.md`

F419E已用compiler-current真实artifact证明：同一个FileIR type address可以合法拥有public API path
`Failure` 与implementation source path `main.Failure` 两个implementation links。当前
`public_artifact_identity_for_addr` 把所有同址link都当成public candidate，错误报告ambiguous。本节点只修
这个production owner并解除最后一个runtime fixture。

权威错误语义继续来自：

- `P5-F280-open-service-error-channel-implementation-audit-result.md`

公开、PublicNameable、SchemaClosed且可编码的错误应以其PackageSchema public identity出界；实现路径不是
第二个公开身份。

## 精确起点与独占范围

- integrated start：
  `b611fe32b9814a0cef07550a1f3cfe7ef4f8333e`；
- F419E test fixture checkpoint：
  `722c9070469aad98af5f38e515756d3897b0f4e4`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时证明三个commit均为HEAD ancestor。

唯一允许写入：

```text
runtime/eval/src/assembly_execution/service_error_channel.rs
runtime/eval/src/assembly_execution/service_error_channel/tests.rs
本任务 result
```

若同模块已有更窄的内联tests，可只在上述文件内放置。禁止修改compiler、artifact model/identity、
linker、shared test support、F419E fixture、其它production或设计；不得派子 Agent、merge/rebase/push、
stable/live。

## 必须实现

`public_artifact_identity_for_addr` 的reverse lookup必须以公开schema owner为事实源：

```text
PackageSchemaIndex entry
  -> entry.public_path
  -> implementation_links.types[public_path]
  -> exact FileIR identity + type index
  -> PackageSchema entry stable key/type id
```

约束：

1. implementation-only source path（如`main.Failure`）即使与公开链接同址，也不能形成第二个public
   candidate。
2. 恰好一个public schema identity匹配地址时返回该identity。
3. 没有public schema identity匹配时返回`None`；private/non-nameable错误仍按既有规则转固定
   InternalError。
4. 两个不同public schema identities真实映射到同一execution address时继续fail closed为ambiguous；
   不能简单按地址任选、按字符串排序或去掉检查。
5. schema entry、public path、implementation link、record owner/key/type-id或exact coordinate损坏时继续
   `InvalidArtifact` / protocol fail closed；不能把损坏artifact当private error。
6. `ServiceErrorTypeIndex`、public export/import、opaque forwarding与每跳stack语义不变。

至少增加回归覆盖：

- 一个public link + 一个同址implementation source link接受，并恢复public `Failure` identity；
- implementation-only同址link不影响结果；
- 真正不同public identity同址仍拒绝（若assembly admission已更早拒绝，保留/引用其动态负例并在本函数级
  构造可执行负例）；
- missing/forged public path/link继续拒绝。

## 验证与交付

使用共享target，先listing再执行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-eval service_error_channel -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-eval service_error_channel
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-eval \
    assembly_execution::ordinary::tests::source_inline_effect_e2e::source_inline_service_effect_sequence_typed_throw_is_caught_then_responds \
    -- --exact
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-eval assembly_execution
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-eval --lib
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo check --locked -p skiff-runtime-eval
cargo fmt --all -- --check
git diff --check
```

目标是typed-throw exact `1/1`、focused `92/92`、full eval `216/216`。写
`P5-F419F-service-error-public-link-reverse-lookup-result.md`，记录exact commit/tree、旧/新candidate
算法、正负例、F419E最后一项闭合、计数与边界。提交并保持clean；不merge/rebase/push。发现需要越过
授权production owner则返回`TASK_SCOPE_EXPANDED`。
