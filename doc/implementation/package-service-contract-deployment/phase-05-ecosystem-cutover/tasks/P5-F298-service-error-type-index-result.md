# P5-F298 Assembly service error type index结果

状态：Implemented checkpoint。

任务提交：`c11b94edb209357f0747979296cd6ff3b3041b11`。

集成提交：`3ac1dd8264d3118586f088bad155e5615d50653b`。

## 直接任务与权威链

- `P5-F298-service-error-type-index.md`
- 任务继续引用F297、F284与F280父链。

## 结果

- loader按exact `PackageSchemaIndexRef`加载并验证完整index、record refs、
  descriptor closure与content identity；
- 删除从operation contract `errors`收集schema roots的旧路径；
- `AssemblyExecutionImage`持有唯一immutable `ServiceErrorTypeIndex`：
  - exact declaration/union-branch execution key映射到类型自己的Package owner、
    stable schema key、type id及canonical record/context；
  - 完整public identity反向映射到execution address set和exact declaration/branch context；
- 保留named-union enclosing/branch、representation owner和同identity多execution address；
- 不依赖runtime-model `CatchIdentity`，不实现编码、`InternalError`、request stack或shape/display
  fallback；
- generic public/PackageSchema error继续fail closed。

## Fail-closed证据面

覆盖missing/mismatched index、record或link，owner/key/id/hash篡改，duplicate type id/public
path，conflicting record，multi-identity address，descriptor mismatch，generic public/applied
PackageSchema，cross-package closure/cycle，同path或shape不同owner，以及operation contract没有
`errors`仍从Package index建表。

## 验证

- loader list/full：PASS，14/14；
- linked-program list/full：PASS，30/30；
- narrow linker service-error-index owner：PASS，4/4；
- `git diff --check`：PASS；
- 标准linker list/full：在枚举前被F300负责的linked throw/call required site与required
  catch旧consumer遮挡，均exit 101。

该检查点解除F300；F298与F299最终共同进入`A5-runtime-channel`独立验收。

