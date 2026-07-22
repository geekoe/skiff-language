# P5-F24B：Service Value Contract Plan

依赖R25 PASS。与F24D并行；独占`runtime/boundary/**`production matcher/codec与直接tests，不改eval、linked-type-plan、
Router或artifact-model owner。一个clean commit。

从pinned `ContractTypeRef + boundary_schema + F24A shape spec`编译唯一service-value plan，同一plan供detached matcher与
canonical WS binary/JSON codec消费。完整支持Event/Result及nested shapes、safe integer、Duration、bounded Date、
JsonObject、representation-over-string Map key与合法contract recursion；拒绝缺/多字段、错tag、unsafe integer、reserved
legacy Json metadata、callback/interface/cycle/alias与foreign schema。null/nominal/zero-byte presence按expected type而非长度。

不得绕过detached clone或在eval复制shape。直接tests覆盖Event connect/receive、Result accept/reject、nominal record/
Duration/JsonObject/map双向detach及完整mutation；跑boundary精确tests、fmt/diff-check，不跑real smoke/full/I16/Host/stable。
