# P5-F279 Open service error channel design result

状态：Design decision complete；解除实现审计节点。

## 直接父节点与权威链

- 被取代的诊断结果：
  `P5-F274-package-typed-throw-projection-audit-result.md`
- 唯一内部架构事实源：
  `doc/architecture/package-service-contract-deployment.md` §3、§4、§6.3、§10
- 用户可见语言事实源：
  `doc/reference/static-semantics.md` §5、§12、§16，
  `doc/reference/runtime.md` §7，
  `doc/reference/std-surface.md` §3、§4
- publication与观测事实源：
  `doc/reference/publication.md` §7，
  `doc/reference/publication-api-yml.md` §8，
  `doc/reference/observability.md` Event Shape

若本文与上述权威文档冲突，以上述文档为准。叶子任务启动时只读其直接父任务；需要依据时沿引用链向上读取。

## 已确认语言语义

- 任意用户`type`声明的名义值都可以`throw`并以其runtime identity被`catch`；不需要
  `ErrorPayload` marker。
- 未被名义`type`包装的primitive、anonymous record、container、interface、`unknown`、function value和
  无约束类型参数不能直接作为catch leaf；透明alias按RHS展开。
- `Exception<E>`是request-local运行时外壳。每个throw都有source location和stack；
  同一request中的rethrow保留原envelope。
- 可抛出不等于可序列化。package内部throw不要求`SchemaClosed`。
- 函数签名、interface operation、Package Local ABI、Publication ABI和ServiceContract都不声明或推导
  operation-specific throw set。实现新增一种可能错误不改变operation signature或
  ServiceProtocolIdentity。

## 已确认跨服务语义

- 实际错误类型在自己的Package owner中显式public、`PublicNameable`、`SchemaClosed`且编码成功时，
  service error envelope携带其`packageId + stableSchemaKey + PackageSchemaTypeId`和payload。
- 错误可以由throwing service的dependency package声明；公开性与identity只看类型自己的owner。
- 接收方链接同一type identity时恢复原名义值。没有链接该类型的中间service不能结构化猜测或catch它，
  但未处理时可以继续转发同一已编码envelope。
- 私有、不可name、非closed或编码失败的用户错误第一次越过service boundary时，转换成固定、公开、
  schema-closed且可捕获的`std.service.InternalError`。原type identity、字段和显示字符串不得出界。
- `InternalError`包含固定脱敏message、`traceId`和唯一`errorId`。中间service未捕获时继续传播同一错误值
  和关联identity，不再套一层其它错误。
- 跨service不序列化callee的`Exception<E>`。caller在service call site创建新的本地exception stack并加入
  脱敏remote-boundary frame。因此B未处理A的错误时，B的caller得到相同错误值，但得到当前这一跳的新栈。
- 每一跳完整本地栈只进入受限telemetry/log，通过`traceId/errorId`关联；wire不能泄露私有source path、
  function name或原始私有错误。
- InProcessBoundary与未来RemoteBoundary必须语义一致。

## 对旧实现方向的修正

P5-F274建议增加declared `throws`语法并填充`PackageCallableSignature.throw_types`，现已被本结果取代：

- 删除`ErrorPayload` prelude/interface/conformance要求及标准错误上的implements；
- 删除artifact/compiler中只服务于公开throw set的`throw_types`和
  `BoundaryOperationContract.errors`；
- 保留真正用于控制流或boundary安全的throw payload provenance，但不能将其解释为公开错误集合；
- 用一个固定开放error envelope承载公开typed error、`std.service.InternalError`和platform error；
- ServiceProtocolIdentity不包含实现可能抛出的类型集合。

Skiff尚未发布，不增加旧artifact、旧prelude或旧wire的兼容读取、双写或fallback。

## 实现前必须审计的owner

下一节点只读确认以下生产链及遮挡关系：

- parser/source type-check与catch leaf owner；
- prelude registry、std源码、syntax/tooling残留；
- artifact model、identity、validation、projection与dependency ingest中的`throw_types/errors`；
- runtime user exception、typed payload materialization、service response error wire及中间转发；
- source location/stack capture、remote-boundary frame、telemetry redaction与`traceId/errorId`；
- test runner inline effects/doubles及公开生态fixtures。

审计必须把共享schema/wire检查点与compiler、runtime、tooling consumer分开，给出最早风险探针和不重叠写入
范围；不得在审计节点修改production代码。
