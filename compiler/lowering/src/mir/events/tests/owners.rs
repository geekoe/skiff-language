use skiff_compiler_source::ExpressionOwnerKey;

use super::{build_model, function};
use crate::mir::{MirSourceEventCollector, MirSourceEventUnavailableReason};

#[test]
fn zero_event_owner_is_available_but_wrong_owner_is_not() {
    let module = "internal.empty_events";
    let sources = [(
        "internal/empty_events.skiff",
        module,
        r#"
          function empty() -> void {}
        "#,
    )];
    let model = build_model(&sources);
    let owner = ExpressionOwnerKey::Function("empty".to_string());
    assert!(model.source_events().contains_owner(module, &owner));

    let lowered = crate::lower(&model).expect("zero-event owner lowers");
    assert_eq!(
        function(&lowered, module, "empty")
            .source_event_plan
            .events(),
        Some(&[][..])
    );

    let wrong = MirSourceEventCollector::new(
        "internal.wrong_owner",
        Some(owner),
        Some(model.source_events()),
    )
    .finish()
    .unwrap();
    assert_eq!(
        wrong.unavailable_reason(),
        Some(MirSourceEventUnavailableReason::SourceOwnerNotProvided)
    );
    assert!(wrong.events().is_none());
}
