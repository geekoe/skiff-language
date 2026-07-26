# P5-F420G3B HTTP-only tooling smoke closure result

状态：`PASS`。本节点已把 generation/ecosystem tooling owner 收敛到 RuntimeAssembly v2
HTTP-only 终态；三个 G3A 识别出的范围外 synthetic v1 fixture 保持不变，终态 owner 反搜为 0。

## 1. Exact start 与 implementation candidate

- integrated start / tree：
  `bc0925396261812cfc9f5bee07246eda14cdff6c` /
  `3b48111e724d722a6ee040682515fcc1b1225cbb`；
- task checkout / tree：
  `1e26dea8b31327017bd0eb8bd7688509cdd084f0` /
  `202d99106e7e6ccfd996e198dc3fb6c1fa4c823b`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`；
- implementation commit / tree：
  `ca837ec343a6ae355bbff77722be9bc827994e05` /
  `ef30feb6e78cc9d65667def00557972f2a72825e`。

`git merge-base --is-ancestor` 对 integrated start 与 accepted F415 均返回 0。task checkout
相对 integrated start 只增加 G3B task 文档。result-only commit / tree 由交付消息记录；它不改变
上述 executable candidate。

## 2. `/case-0` producer / consumer 一致性

Rust current producer
`test-runner/src/package_test_assembly.rs` 仍生成
`test.skiff/package/<safe-coordinate>/case-{index}`。本节点没有修改 producer，而是把本批三个
consumer 统一到 case index 0：

1. shared receipt oracle 按 Rust 相同的 ASCII safe-coordinate 字符规则计算
   `test.skiff/package/<safe-coordinate>/case-0`；
2. shared synthetic fixture 使用
   `test.skiff/package/test.skiff/package-service-websocket-smoke/case-0`；
3. I02 synthetic fixture 使用
   `test.skiff/package/test.skiff/package-service-i02-spawn-submit/case-0`。

shared oracle 继续要求 schema `skiff-package-service-smoke-fixture-v2`、精确两个 entrypoint、
精确 HTTP selector/key/identity/mode 和无额外字段；缺失、第三项、v1 以及旧
`contract` / `operation` 字段均 fail closed。

## 3. 删除、迁移与保留

删除的旧 CLI / module：

- `scripts/run-package-service-generation-lifecycle-smoke.mjs`；
- `scripts/lib/package-service-generation-lifecycle-smoke-real.mjs`；
- `scripts/lib/package-service-generation-lifecycle-smoke-oracle.mjs`。

删除的六个旧 test：

- `scripts/tests/package-service-ecosystem-smoke-lifecycle.test.mjs`；
- `scripts/tests/package-service-ecosystem-smoke-real.test.mjs`；
- `scripts/tests/package-service-generation-lifecycle-fixture-combined.test.mjs`；
- `scripts/tests/package-service-generation-lifecycle-smoke-lifecycle.test.mjs`；
- `scripts/tests/package-service-generation-lifecycle-smoke-oracle.test.mjs`；
- `scripts/tests/package-service-generation-lifecycle-smoke-real.test.mjs`。

`scripts/lib/package-service-ecosystem-smoke-real.mjs` 只保留 I02 与 actor acceptance 当前消费的
fixture Cargo args helper；Assembly-WebSocket runner、`ws` loader、marker、deadline 和
WebSocket lifecycle exports 均已删除。

I02 仍需要的 HTTP request 语义迁入新 owner
`scripts/lib/package-service-http-unary.mjs`：POST / isolated HTTP ingress 校验、512-byte bounded
raw body、oversize fail-closed、非 200 wire metadata、bounded/redacted body diagnostic，以及
RuntimePayload string decode / exact value validation。I02 直接从该中性 owner import，不再依赖
generation module。

保留的 current coverage / owner：

- `scripts/tests/package-service-ecosystem-http-fixture.test.mjs` 继续证明 v2 两个 HTTP
  entrypoint 与旧字段 fail-closed；
- 新增 `scripts/tests/package-service-http-unary.test.mjs`，保留 current HTTP utility coverage；
- `scripts/tests/package-service-i02-combined.test.mjs` 继续证明 I02 commit、零 artifact request
  I/O 与 rollback；
- actor acceptance 与 I02 继续共用 fixture Cargo args；
- Router 通用 WebSocket、connection lifecycle、generation registry，以及 G3A 指定的三个范围外
  synthetic v1 fixture均未修改。

## 4. 聚焦验证

在 implementation commit 上运行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
node --test \
  scripts/tests/package-service-ecosystem-http-fixture.test.mjs \
  scripts/tests/package-service-http-unary.test.mjs \
  scripts/tests/package-service-i02-combined.test.mjs
```

结果：`15 / 15 PASS`，其中 ecosystem HTTP fixture 4 个、HTTP unary utility 5 个、I02 6 个。
HTTP utility focused coverage包含成功 raw bytes、oversize 截断、非 200 metadata、secret/path
脱敏与 canonical / malformed / truncated RuntimePayload decode。

```bash
node scripts/verify.mjs --only tooling --list
```

结果：基线 `57` phases，删除六个旧 test并新增一个 HTTP utility test后精确为 `52` phases，
即 `57 - 6 + 1 = 52`。

两组合同反搜：

```bash
rg -n \
  "runPackageServiceGenerationLifecycleSmoke|r05-generation-lifecycle|entrypoints\\[2\\]" \
  scripts --glob '*.mjs'

rg -n "skiff-runtime-assembly-v1" scripts \
  --glob 'package-service-ecosystem-smoke-*.mjs' \
  --glob 'package-service-generation-lifecycle-*.mjs' \
  --glob 'run-package-service-generation-lifecycle-smoke.mjs'
```

结果均为 0 命中。`git diff --check` PASS。

## 5. 边界与工作区

没有修改 Router、Rust/test-runner、fixture source、manifest、lockfile、verify plan、三个范围外
v1 synthetic fixture或其它 test。没有运行完整 tooling、其它 N4 gate、stable 或 live；没有
启动 instance/watch registry，也没有 merge、rebase 或 push。implementation 提交前后均未发现
范围扩张；result 提交后 worktree clean 由交付消息确认。
