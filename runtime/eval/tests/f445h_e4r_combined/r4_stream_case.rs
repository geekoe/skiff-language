use super::{execution_control::*, execution_harness::*, imports::*, poll_support::first_poll};

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn f445h_e4r_combined_r4_stream_observes_child_scope_and_cleans_non_end() {
        let file = Arc::new(LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: "file:f445h-e4r-combined-stream".to_string(),
            source_ast_hash: "source:f445h-e4r-combined-stream".to_string(),
            module_path: "combined.stream".to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: SourceMapDto::default(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            actor_declarations: Vec::new(),
            types: Vec::new(),
            constants: Vec::new(),
            executables: vec![LinkedExecutable {
                kind: ExecutableKind::Function,
                symbol: "combined.stream.consume".to_string(),
                type_params: Vec::new(),
                params: Vec::new(),
                return_type: None,
                self_type: None,
                slots: SlotLayoutIr {
                    slots: vec![SlotIr {
                        index: 0,
                        name: "item".to_string(),
                        kind: "local".to_string(),
                        writable_local: false,
                    }],
                    frame_size: 1,
                },
                may_suspend: true,
                body: LinkedExecutableBody {
                    blocks: vec![BlockIr {
                        label: "body".to_string(),
                        statements: Vec::new(),
                    }],
                    statements: Vec::new(),
                    expressions: Vec::new(),
                },
            }],
            external_refs: ExternalRefTable::default(),
        });
        let (interpreter, stream) = interpreter_for(Arc::clone(&file));
        let deadline = Instant::now() + Duration::from_secs(5);
        let (control, child_scope) = HarnessControl::child(deadline);
        let context = execution_context(&interpreter, control, HarnessConfig::ordinary());
        let addr = ExecutableAddr {
            unit: UnitAddr::Service,
            file: FileAddr::FileIrIdentity(file.file_ir_identity.clone()),
            executable: 0,
        };
        let heap = RequestHeap::default();
        let mut env = Env::new();
        let stream_value = stream_value("f445h-e4r-combined-pending-stream");
        let mut access = skiff_runtime_eval::heap_access::HeapAccess::private(heap);
        let mut execution = Box::pin(interpreter.exec_program_stream_for_in(
            context,
            &mut access,
            &mut env,
            &addr,
            &file,
            &file.executables[0],
            0,
            "body",
            stream_value,
            None,
            &[],
            None,
        ));

        assert!(
            matches!(first_poll(execution.as_mut()), Poll::Pending),
            "combined stream reaches the real pending next()"
        );
        assert!(matches!(
            child_scope.terminal_at(deadline),
            Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
        ));
        let outcome = tokio::time::timeout(Duration::from_millis(250), &mut execution).await;
        let completed_from_child_scope = outcome.is_ok();
        let terminal = outcome
            .as_ref()
            .ok()
            .map(|result| format!("{result:?}"))
            .unwrap_or_else(|| "harness timeout while stream next remained pending".to_string());
        drop(execution);
        assert_eq!(
            stream.state.cleanup_cancels.load(Ordering::Acquire),
            1,
            "non-End stream termination must run exactly one consumer cleanup"
        );
        assert!(
        completed_from_child_scope,
        "R4 expected current child scope to terminate pending next() before cleanup; {terminal}; next received {} cancellation token(s)",
        stream
            .state
            .last_cancel_token_count
            .load(Ordering::Acquire)
    );
    }
}
