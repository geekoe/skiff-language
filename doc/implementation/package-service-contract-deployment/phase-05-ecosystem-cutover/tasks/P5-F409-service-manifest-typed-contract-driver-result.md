# P5-F409 Service manifest typed contract and driver result

状态：Complete（F409 scope）。

## 1. 提交锚点与范围

| 锚点 | commit | tree |
| --- | --- | --- |
| 任务规定 start checkpoint | `be2a1a8a7b893a44c98918114529f32d18ea963c` | `a81a25c63f1060571dc554bc257311910a792aa5` |
| task definition / worktree start | `4d3b7274972bb5cff2837a38b77e5f637eb585e0` | `fb41351b4132e1c5db26afbf5095dcc4c06fee3f` |
| implementation end | `2026b0e6831285c052126abba93d079034e8afac` | `58493c7752cf313ae2346d7dd9b0e20b1bf683c7` |

实现提交：

```text
2026b0e6831285c052126abba93d079034e8afac
feat(compiler): project service manifest selection
```

生产代码只修改任务授权的`compiler/contract/**`和`compiler/driver/**`；测试、Cargo target与fixture
只修改任务列出的compiler范围。没有修改artifact model/identity、F408 producer/parser、
deployment、runtime、router、test-runner、ecosystem source或权威设计，也没有merge、rebase或push。

## 2. Typed manifest selection

`compiler/contract/src/selection.rs`现在是`service.yml.serviceCalls`字符串到typed callable的唯一owner。
内部`ServiceCallSelection`保存：

- duplicate检查之前不做dedup；任一重复public root结构化拒绝；
- canonical sorted roots；
- `stable operation path -> exact PackageCallableId`的`BTreeMap`。

每个root只按Package Local ABI完整public graph解析：

1. 先索引全部`PublicInstance.methods`，因此精确method path（如`worker.run`）即使同时存在Callable
   symbol也会拒绝，要求选择`worker`root。
2. ordinary Callable只有在exact ID不属于任何public-instance method时才进入operation map；
   指向method exact ID的另一个public alias结构化拒绝。
3. PublicInstance展开其全部listed methods；每个`root.method`必须存在同exact ID的public
   Callable symbol。
4. Type、Constant与unknown path分别结构化拒绝。
5. 任意两个展开后的operation path若映射同一exact callable，结构化拒绝。

`project_service_api`显式接收selection paths，不读取PackageArtifact selection，也不从deployment或
runtime反推。它只对selected exact callable读取boundary projection：

- `Available`进入operation map和既有Package schema reachable closure；
- 全部selected `Unavailable`及其完整reasons按operation path一次聚合；
- missing projection fail closed。

`ServiceApiProjection.service_calls`携带contract owner产出的canonical roots；
`available`继续保存stable operation path到exact callable的唯一map。Package API visibility仍列出
全部public Callable，只有selected Available callable带`service_operation_id`。未选中的Available
callable保留Package public visibility但不进入ServiceContract。

missing selection与`[]`都生成合法、稳定的zero-operation ServiceContract，不产生schema requirement。
ServiceContract保持v4，service protocol identity保持v4，既有schema closure算法未改。

## 3. Driver与generated deployment单一流

driver现在只有一条Package编译路径：

```text
compile_package
  -> canonical PackageArtifact
  -> compile_service_package读取validated service_root.service.service_calls
  -> project_service_api
  -> ServiceApiProjection
  -> generated deployment input
```

具体收敛：

- 删除`compile_package_with_service_call_roots`、`service_call_paths`、ordinary/service marker gate和
  旧builder测试；`compile_package`始终只编译Package。
- `compile_service_package`调用同一个`compile_package`后，再显式传入manifest selection。
- generated operation input直接clone `ServiceApiProjection.available`中的exact ID：

  ```text
  ServiceDeploymentOperationInput {
    contract_operation_id,
    package_callable_id
  }
  ```

  不再扫描Local ABI public path，也没有`package_public_path`或fallback。
- generated boundary先把原manifest roots canonical sort并与typed
  `ServiceApiProjection.service_calls`精确比较；duplicate或不一致fail closed。
- `generated_revision`只写入typed projection的canonical roots，因此原YAML数组顺序不进入revision。

## 4. Identity正反例

| 场景 | PackageArtifact / build / Local ABI | ServiceContract / protocol | deployment revision / identity |
| --- | --- | --- | --- |
| `[selected, worker]` vs `[worker, selected]` | 完全相同 | 完全相同 | 完全相同 |
| missing vs `[]` | 完全相同 | 同一zero-operation contract | 完全相同 |
| `{read}` vs `{read, write}` | 完全相同 | operation set与protocol identity变化 | bindings、revision与deployment identity变化 |
| compatible source rebuild、同operation contract | build变化 | protocol identity不变 | implementation/deployment变化 |
| 同一source同时被service operation与HTTP handler引用 | 同一source target | contract operation identity独立 | gateway identity独立；两个callable domain均保留 |

generated deployment断言operation binding中的`packageCallableId`与
`ServiceApiProjection.available`的exact ID相同，wire中没有`packagePublicPath`。

## 5. Fixture与generation同步

- test target从`service_call_roots`改名为`service_calls_manifest_selection`，旧文件删除。
- owned `api.yml {source, serviceCall}`全部改为scalar selector，selection移入对应`service.yml`。
- owned PackageArtifact literals删除`service_call_roots`，固定使用v8 schema；owned contract fixture切到
  ServiceContract/protocol v4。
- canonical std build pin固定为当前完整v9事实：
  `skiff-package-build-v9:sha256:8ac1d3ee235fb3f543df52430f1539610ca05c5631a09df22f7c4f4a7b6a8e17`。
- generated deployment、HTTP dual-surface和stream conformance fixture都经过真实Package+Service root。
- service conformance fixture补齐当前canonical rules要求的database state、exact PackageSchema package
  dependency和exact package nominal value；这些都是fixture修正，没有修改production owner。

## 6. 反向搜索

以下旧selection identifier在compiler中为零：

```text
PackageServiceCallRoot
service_call_roots
compile_package_with_service_call_roots
service_call_paths
with_service_call（exact identifier）
.service_call
```

`compiler/driver/generated_deployment.rs`和对应integration test中的snake-case
`package_public_path`为零。唯一`packagePublicPath`是
`generated_service_deployment.rs`的wire absence断言，不是reader、writer或fallback。

owned范围内旧ServiceContract/protocol、v7 PackageArtifact和v8 Package build literal为零。全compiler
仍有两处明确的F408 fail-closed negative probe：

```text
compiler/projection/src/package_artifact/tests/projection.rs
  stale skiff-package-artifact-v7 rejection
  stale skiff-package-build-v8 prefix rejection
```

`serviceCallRoots`仅剩F408 wire absence断言；旧`api.yml serviceCall`字符串仅剩F408 strict parser
negative tests。两者都不是authoring reader兼容层。

`\bservice_call\b`唯一命中`compiler/lowering/src/external_refs.rs`的test helper；另有
`service_call_refs`、`service_call_ref_closure`、`file_ir_service_call_sites`和
`validate_file_ir_service_calls`共41处命中任务查询范围，全部属于必须保留的File IR call-site lowering、
artifact refs与conformance断言，不是manifest selection。

## 7. Test discovery与执行

所有required selector都先运行`-- --list`，再运行同一最终代码状态。

| selector | `-- --list`实际选择 | 执行 |
| --- | ---: | --- |
| `skiff-compiler-contract projection` | 4 | 4 passed |
| `skiff-compiler --lib pipeline` | 12 | 12 passed |
| `service_calls_manifest_selection` | 5 | 5 passed |
| `generated_service_deployment` | 11 | 11 passed |
| `http_gateway_projection` | 8 | 8 passed |
| `service_conformance` | 14 | 14 passed |
| `artifact_model_conformance` | 1 | 1 passed |
| `builtin_canonical_spelling` | 2 | 2 passed |

required executable tests合计`57 passed / 0 failed`。额外执行：

- `skiff-compiler --lib authoring`：`8 passed / 0 failed`；
- `skiff-compiler --lib canonical_dependencies`：`3 passed / 0 failed`。

静态与质量检查：

```text
cargo check --locked -p skiff-compiler                 PASS
cargo fmt --all -- --check                             PASS
git diff --check                                       PASS
```

所有Cargo测试/check命令使用任务指定的共享
`CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target`。

任务原样的无target限定`cargo test --locked -p skiff-compiler pipeline -- --list`会在selector开始前编译
所有explicit integration targets，并被范围外
`compiler/tests/actor_dispatch_linking.rs`仍写`RuntimeAssembly.global_ingress`的既有错误阻断。
本任务没有越界修改该文件；使用真实lib selector `--lib pipeline`发现并执行12个非零测试。无target限定
命令留给integration tree在对应owner同步后重跑。

没有运行完整workspace/isolated/stable/live、instance或生态publish。

## 8. 自验收矩阵

| 任务条款 | 代码/测试证据 | 结论 |
| --- | --- | --- |
| contract是唯一typed selection owner | `selection.rs` canonical roots + exact operation map；driver只消费projection | PASS |
| ordinary function / complete public instance | real manifest fixture得到`selected`与`worker.run/stop` | PASS |
| direct method / Type / Constant / unknown | structured negative matrix | PASS |
| method alias / duplicate exact callable | synthetic complete-public-graph negative probes | PASS |
| missing boundary / aggregate Unavailable | selected boundary negative matrix | PASS |
| zero-operation稳定 | missing与`[]` contract/deployment equality | PASS |
| Package visibility不受selection裁剪 | unselected Available保留且无operation ID | PASS |
| Package identity不含selection | subset/superset artifact、build、Local ABI完全相同 | PASS |
| protocol/deployment随operation set变化 | subset/superset identity matrix | PASS |
| manifest数组顺序不入identity | direct contract API与generated deployment双层order probe | PASS |
| generated exact callable binding | input/output exact ID一致；无public-path writer | PASS |
| HTTP与service operation domain分离 | dual-surface真实fixture | PASS |
| 旧模型删除与call-site facts保留 | 反向搜索分类 | PASS |

结论：F409 typed selection、ServiceContract projection与compiler driver producer完成；解除F412/F413和
F414前置，但不宣称范围外integration target或整个阶段已经验收。
