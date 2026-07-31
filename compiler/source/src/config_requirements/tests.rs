use super::*;

#[test]
fn own_usage_keeps_presence_and_deduplicates_identical_reads() {
    let seed = ConfigUsageSeed {
        typed: vec![
            ConfigUse {
                path: "provider.apiKey".to_string(),
                ty: "string".to_string(),
                required: true,
                source_path: "first.skiff".to_string(),
                source_span: None,
            },
            ConfigUse {
                path: "provider.apiKey".to_string(),
                ty: "string".to_string(),
                required: true,
                source_path: "second.skiff".to_string(),
                source_span: None,
            },
        ],
        presence: vec![ConfigPresenceUse {
            path: "provider".to_string(),
            source_path: "first.skiff".to_string(),
            source_span: None,
        }],
    };
    let requirements = ConfigRequirementSet::from_usage_seed(&seed);
    assert_eq!(requirements.requirements().len(), 2);
    assert!(requirements
        .requirements()
        .iter()
        .any(|requirement| requirement.access().is_has()));
    assert_eq!(
        requirements
            .requirements()
            .iter()
            .find(|requirement| requirement.path() == "provider.apiKey")
            .unwrap()
            .provenances()
            .len(),
        2
    );
}
