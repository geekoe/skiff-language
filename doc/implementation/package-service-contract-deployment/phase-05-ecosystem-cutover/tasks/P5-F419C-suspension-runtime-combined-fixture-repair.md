# P5-F419C Suspension runtime combined fixture repair

状态：Ready。

## 直接父节点

- `P5-F419A-suspension-consumer-combined-gate-result.md`

父门禁已证明 runtime production、unified lane、typed stream deadline与combined compile正确。本节点只修复
同一组8个旧 test fixture，不能修改production。

## 精确起点与独占范围

- gate result commit：
  `087469235de2d1bb67965bce884b963d537c3f47`；
- failed production candidate：
  `2b9d29eea9a65ab323240f1e6c34b3e3b29c7403`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时证明三个commit都是HEAD ancestor。

唯一允许写入：

```text
runtime/eval/src/assembly_execution/ordinary/tests.rs
runtime/eval/src/assembly_execution/ordinary/tests/source_inline_effect_e2e.rs
本任务 result
```

禁止修改production、validator、artifact model、compiler、deployment、tooling、设计或其它fixture；不得派
子 Agent、merge/rebase/push、stable/live。

## 精确修复

1. `package_direct_fixture_with_caller` 的callee package id为
   `example.package-direct-callee`，public path为`mutate`；fixture必须使用当前canonical
   `PackageCallableId`：

   ```text
   pkg-callable:example.package-direct-callee:mutate
   ```

   同一exact id必须用于PackageArtifact public symbol、semantic facts、implementation link、caller
   external ref与package direct link。不得放宽canonical callable id validation。
2. `write_consumer_package` 与 `write_std_effect_consumer_package` 需要一个合法但无公开项的API控制文件；
   写 canonical empty map（`{}\n`），不能写零字节文件，也不能移除`api.yml`或放宽parser。
3. 除上述current authoring/identity适配外不改变8个测试的业务语义、expected effects、same-heap、
   typed throw、stream顺序、request subset或diagnostic断言。

## 验证与交付

使用共享target，先listing再执行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-eval assembly_execution -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-eval assembly_execution
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-eval --lib -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-eval --lib
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo check --locked -p skiff-runtime-eval
cargo fmt --all -- --check
git diff --check
```

预期实际数量为 focused `92`、full eval `216`，且父门禁的同一8个fixture全部通过。写
`P5-F419C-suspension-runtime-combined-fixture-repair-result.md`，记录exact commit/tree、三项机械适配、
8个测试闭合、实际计数和边界。提交并保持clean；不merge/rebase/push。发现需要production修改则停止并
返回`TASK_SCOPE_EXPANDED`。
