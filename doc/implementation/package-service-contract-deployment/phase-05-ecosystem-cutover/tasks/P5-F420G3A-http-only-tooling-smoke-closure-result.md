# P5-F420G3A HTTP-only tooling smoke closure result

状态：`TASK_SCOPE_EXPANDED`。预检发现任务要求的全 `scripts/**/*.mjs`
旧残留反搜与三个未授权、既有的 synthetic RuntimeAssembly v1 test fixture 冲突。本节点在任何
implementation 写入前立即停止；没有 implementation commit，也不宣称 HTTP-only tooling smoke
已闭合。

## 1. Exact start 与停止点

- integrated start / tree：
  `1010929ed2508d3b5d4bfcd1537d4eef3c599aa3` /
  `7be5b76d7a234c731fef9044a772b118951da3b9`；
- task checkout / tree：
  `dfc2668f86eba286ad280614f6cf801ee9fe7fab` /
  `e0c6dcc78191bf52b9b4762f6fb8c697eeeb8249`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

`git merge-base --is-ancestor` 对 integrated start 与 F415 均返回 0。task checkout 相对
integrated start 只增加修正版 G3A task 文档，没有 executable diff。最终 result-only commit /
tree 由交付消息记录；它同样不改变 executable candidate。

## 2. 精确范围冲突

任务要求最终运行以下反搜且结果精确为 0：

```bash
rg -n \
  "runPackageServiceGenerationLifecycleSmoke|r05-generation-lifecycle|entrypoints\\[2\\]|skiff-runtime-assembly-v1" \
  scripts --glob '*.mjs'
```

在 exact integrated start 上执行等价的 `git grep`，除了本任务本来就会删除的
generation/ecosystem owner，还得到三个不在允许写入集合内的
`skiff-runtime-assembly-v1` 命中：

```text
scripts/tests/isolated-test-runtime.test.mjs:186
scripts/tests/package-service-host-negative-probe.test.mjs:14
scripts/tests/skiff-test-cli.test.mjs:26
```

三者都是早于本任务存在的 synthetic test identity，不是旧 generation/ecosystem tooling lane：

1. `isolated-test-runtime.test.mjs` 用该 identity 关联 synthetic bootstrap、active assembly、
   replica 与 capability，验证 isolated runtime readiness tuple；
2. `package-service-host-negative-probe.test.mjs` 把它作为 command-double
   `readHostReceipt` 返回的 synthetic base assembly，验证 Host negative probe；
3. `skiff-test-cli.test.mjs` 把它作为 fake Cargo CLI invocation 的 synthetic
   `--base-assembly` 参数，验证 `skiff test` 参数传递。

这三个 test 均不是任务允许删除的六个旧 test files，也不是允许迁移的 I02/shared fixture
owner。任务同时明确禁止修改“其它 test”。因此即使完成允许范围内的删除与 HTTP utility
迁移，给定的全 `scripts` 反搜仍至少保留这三个命中；修改它们以制造 0 结果会违反 write
allowlist。

## 3. 最小 successor 合同修正

后继不应把这三个无关 fixture 纳入 G3 的迁移范围。应把反搜限定到本批删除/迁移的
generation/ecosystem owners，例如对 `skiff-runtime-assembly-v1` 使用 basename glob：

```bash
rg -n "skiff-runtime-assembly-v1" scripts \
  --glob 'package-service-ecosystem-smoke-*.mjs' \
  --glob 'package-service-generation-lifecycle-*.mjs' \
  --glob 'run-package-service-generation-lifecycle-smoke.mjs'
```

`runPackageServiceGenerationLifecycleSmoke`、`r05-generation-lifecycle` 与
`entrypoints[2]` 仍可按 successor 想要证明的 owner 集合做同样的有界反搜。这样证明的是本批
旧 Assembly-WebSocket lane 已移除，不会把 isolated readiness、Host negative probe 或 CLI
argument fixture 偷渡为新的迁移 owner。

不需要因此修改 Router、Rust/test-runner、fixture source、manifest、lockfile、verify plan、
上述三个 test 或任何其它 production owner。

## 4. 停止证据与边界

预检已经完成：

- integrated start/tree 与 F415 ancestry：PASS；
- `node scripts/verify.mjs --only tooling --list`：当前 baseline 为 57 phases；
- exact-start `git grep`：精确定位上述三个范围外既有命中；
- 停止前 `git status --porcelain=v1`：空；
- 停止前 `git diff --check`：PASS。

因为 mandatory terminal reverse search 在 current allowlist 下不可满足，没有修改
package-test service id、没有删除 generation/ecosystem files、没有迁移 HTTP utility，也没有
运行 focused tests 或 post-change tooling list。没有 implementation commit。

本节点没有运行完整 tooling、Cargo fixture、Router、test-runner Rust suite、
`run-skiff-tests`、stable 或 live；没有 merge、rebase、push，也没有操作 instance/watch
registry。
