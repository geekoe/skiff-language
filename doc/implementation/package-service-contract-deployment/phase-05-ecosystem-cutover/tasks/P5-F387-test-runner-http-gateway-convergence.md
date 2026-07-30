# P5-F387 Test-runner HTTP gateway convergence

状态：Ready（从F386 clean checkpoint串行完成T1与T2）。

## 直接父节点

- `P5-F386-package-test-http-gateway-fixture-blocker.md`
- 完整冻结合同：`P5-F384-test-assembly-gateway-control-plane-audit-result.md`
- Router前置：`P5-F385-router-test-gateway-control-plane-result.md`

## Worktree

- `/Users/geek/workspace/skiff-p5-f386-package-test-http-gateway`
- branch `codex/p5-f386-package-test-http-gateway`
- clean HEAD `a2ea3cd40e1f1f262ed4526b319e110f48052e6b`

不得reset/rebase或重写F386 checkpoint。新Agent先审阅已有T1 WIP，再在其上追加完成，不从头重做。

## T1完成要求

按F384 §4.1/4.2/5/7.2完成并验证已有package-test迁移：

- 每case zero-op contract、empty bindings、one gateway/ingress；
- exact private Null→Null wrapper、key/selector/identity/reference closure；
- strict F385 control request与`response.end`/inner HTTP/body验证；
- inline setup effect经test-only true seam真实生效；
- 结构负例和至少一个真实isolated package-test通过。

不得恢复contract/operation/doubles compatibility字段。

## T2 Ecosystem HTTP要求

按F384 §4.3、§6 T2和§7.3完成：

1. ecosystem HTTP projection精确两个gateway entry：
   - package-test `run`，Null→Null identity
     `cfcfced94f984612809ce837f81e975016b09f206925389d95e925e087fc32d4`；
   - smoke `probe`，host `ecosystem-smoke.skiff.localhost`、POST `/probe`、private
     `main.__skiffHttpProbe(body:null)->string`，identity
     `adfaa17c077af0388f2b5751bbe4b9ba392ec647f5ce33022c8e8ec83eaf6653`。
2. 两个contract均zero-op，operation bindings为空；HTTP assembly只有两个root、两个deployment、两个
   gateway ingress。
3. normal fixtures的private wrapper调用原`main.marker()`；I02 wrapper调用当前API映射实际指向的
   `main.submitSpawnReceipt()`。wrapper参与正常source compile且不进入API；不得编译后改写artifact。
4. HTTP receipt升级为`skiff-package-service-smoke-fixture-v2`：
   - 只保存deployment/key/identity/mode/selector；
   - 不保存contract/operation；
   - 不dual-read v1。
5. 完整WebSocket smoke不混入HTTP正例；不得把不可路由WS entry算作第三个HTTP gateway。

## 写入边界

保留F386允许文件，并新增：

- `test-runner/src/ecosystem_smoke_fixture.rs`
- `test-runner/src/bin/package_service_smoke_fixture.rs`
- 四个现有ecosystem fixture的private HTTP wrapper/direct tests
- `scripts/lib/package-service-ecosystem-smoke-oracle.mjs`
- `scripts/tests/helpers/package-service-ecosystem-smoke-fixtures.mjs`
- 对应non-live v2 HTTP receipt test

禁止修改Router/F359/F365、WebSocket协议/production consumer、其它仓库、stable/live。

## 验收

运行F384 §7.2与§7.3全部命令，包括：

```bash
cargo test --locked -p skiff-test-runner --lib runtime_execution -- --test-threads=1
cargo test --locked -p skiff-test-runner \
  --test package_service_contract_deployment -- --test-threads=1
cargo check --locked -p skiff-test-runner --bins
cargo clippy --locked -p skiff-test-runner --all-targets --no-deps -- -D warnings
node --test scripts/tests/package-service-ecosystem-http-fixture.test.mjs
git diff --check
```

若仓库现有Node test名称不同，可新增父合同指定的v2 direct test，但必须枚举非零。真实isolated
package-test必须通过；ecosystem只跑本节点HTTP non-live路径，不操作stable/live。

结果写`P5-F387-test-runner-http-gateway-convergence-result.md`，追加清晰commit，worktree clean；不
merge/rebase/push。新Agent执行，不派子Agent。若必须改WS或共享协议，返回`TASK_SCOPE_EXPANDED`。
