# P4-F07：Canonical Callback Interface Projection

## 权威输入、风险与证据状态

- 执行输入：R02在`ee1609c`的blocking issues 1、4；production把`ContractTypeId`直接当local interface ABI，
  adapter按`BTreeMap`顺序与method-table slot隐式`zip`，host又手工carrier绕过真实projection。
- 风险/验收组：高风险typed identity/operation mapping；由R02复验，不直接解锁Wave 3。
- integration边界：只提交task branch，不merge integration/main、不push。

## DAG 与执行约束

- 依赖：T04–T06合流及R02 FAIL；可与F06并行。
- 解锁：F08。
- branch：`codex/p4-f07-canonical-callback-interface-projection`。
- worktree：`/Users/geek/workspace/skiff-p4-f07-callback-projection`。
- 五分钟内真实edit；原T06 owner执行。若admitted artifact确实缺少建立显式mapping所需的local method name/ABI/slot/
  signature事实，立即报告`TASK_NOT_EXECUTABLE`与最小typed fact owner；不得比较identity字符串或恢复隐式顺序配对。

## 写入范围与完成态

- 独占callback/native lane、native callback adapter、callback所需immutable runtime model/table metadata与owned host
  callback测试；必要时在eval boxing seam保留admitted linked method metadata。不修改ordinary/stream/compiler/router。
- 新建显式`CallbackContractProjection`（名称可内部调整），保持`ContractTypeId`、local interface ABI、method ABI三种
  identity domain分离。projection由真实contract descriptor与admitted/linked local interface method metadata生成。
- operation按stable contract operation key/name显式匹配local method name，再验证exact operation set、contiguous slot、
  method ABI、parameter/return signature与suspend语义；禁止`BTreeMap`/declaration-order `zip`。
- capability carrier的contract字段使用canonical callback contract identity；table payload持有validated projection，调用时
  以carrier contract + requested slot/method ABI解析exact owner executable。
- native显式adapter descriptor也必须声明同样的operation mapping与boundary type；缺mapping/marker/registration fail closed。
- `typed_execution_callback_native`必须经production service materialization hook生成capability，不手工register/carrier；
  与T04合流后断言进入exact owner/provider结果（冻结body可返回missing block entry），wrong mapping/tuple在executable前失败。

## 唯一验证 ownership

```bash
cargo test -p skiff-runtime-model callback_contract_projection
cargo test -p skiff-runtime-eval in_process_callback
cargo test -p skiff-runtime-native callback_adapter
cargo test -p skiff-runtime-host typed_execution_callback_native
git diff --check
```

过滤器必须非零PASS；不得运行stream item或完整runtime gate。

## 回报

提交一个clean commit，回报三identity domain、mapping preimage/validation矩阵、production host链、命令与任何typed fact blocker。
