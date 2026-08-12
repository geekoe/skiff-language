# REV0-S: production VCP seam review

> Disposition: `FAIL`, corrected before Development join
>
> Candidate decision: `decisions/dec0-vcp-production-seam.md` at integration
> commit `afc79c122478c10a2536c484ac3f7292d3f3d99f`
>
> Review mode: independent, read-only; no Cargo command

## Blocking findings

1. The D0-O exact write set omitted
   `runtime/host/src/loader/bytecode_admission.rs`, although
   `BytecodeDeploymentRegistry::route` and `BytecodeRoute::request_target` are
   the sole owners assigned to mint `DeploymentImageSelected` and
   `RouteEntryPinned`.  Moving those events to `assembly_wire` would turn an
   observation into a non-owner assertion.  D0-O also needs the focused host
   route test file after D0-R changes its route interface.
2. D0-R could not remove request-time artifact store reads until the admitted
   image exposed narrow borrowed views of the already sealed service protocol,
   operation, ingress and gateway adapter facts.  A host sidecar, raw candidate
   lookup or identity guess would be a second authority.

The second blocker was already corrected by MAP revision 7, which explicitly
added read-only `VerifiedLinkedBytecodeImage` accessors to D0-R.  The first is
corrected in the reviewed decision alongside this receipt.  D0-O must start
after both D0-R and D0-M have joined.

## Passing conclusions and residual join gates

- The host-internal VCP enters only `RuntimeHost::spawn_bytecode_request` and
  adds no public or test-only executor.
- A `runtime/model` observer contract preserves dependency direction and can
  carry only bounded typed identities and coordinates.
- Observer panic isolation must be executable, and the default telemetry
  projection must remain non-blocking.  A panicking/dropping sink must not
  change response, terminal ownership or cleanup.
- `VmFirstInstructionDispatched` must be emitted one time only after an opcode
  arm returns `Ok`.
- terminal observation follows the winning completion claim and budget finish;
  cleanup follows release of request execution, target, supervised handle and
  route/image request pins, plus an exact matching-row absence check rather
  than a global active count.
- After the two write-set corrections, no additional hard blocker to a green
  host-owned VCP was found.
