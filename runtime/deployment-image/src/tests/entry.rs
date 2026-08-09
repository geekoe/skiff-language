use std::sync::Arc;

use crate::{
    DeploymentImage, DeploymentOwnerIdentity, DeploymentProgramEntry, PinnedDeploymentEntry,
    PinnedDeploymentEntryError,
};

use super::owner;

#[derive(Debug, PartialEq, Eq)]
struct NonCloneProgram {
    label: &'static str,
}

#[derive(Debug)]
struct FakeEntry {
    owner: DeploymentOwnerIdentity,
    program: Arc<NonCloneProgram>,
    name: &'static str,
}

impl DeploymentProgramEntry<NonCloneProgram> for FakeEntry {
    fn owner(&self) -> &DeploymentOwnerIdentity {
        &self.owner
    }

    fn program(&self) -> &Arc<NonCloneProgram> {
        &self.program
    }
}

#[test]
fn entry_pins_the_same_non_clone_program_and_exact_owner() {
    let program = Arc::new(NonCloneProgram { label: "verified" });
    let exact_owner = owner("build:entry");
    let image = Arc::new(
        DeploymentImage::try_new(exact_owner.clone(), Arc::clone(&program), [])
            .expect("empty dependency set is valid"),
    );
    let pinned = PinnedDeploymentEntry::try_new(
        Arc::clone(&image),
        FakeEntry {
            owner: exact_owner.clone(),
            program: Arc::clone(&program),
            name: "gateway-entry",
        },
    )
    .expect("entry and image share the same program allocation");

    let object_safe_entry: &dyn DeploymentProgramEntry<NonCloneProgram> = pinned.entry();
    assert_eq!(object_safe_entry.owner(), &exact_owner);
    assert!(Arc::ptr_eq(object_safe_entry.program(), &program));
    assert!(Arc::ptr_eq(pinned.image(), &image));
    assert_eq!(pinned.owner(), image.owner());
    assert_eq!(pinned.entry().name, "gateway-entry");
}

#[test]
fn entry_rejects_an_equal_program_from_a_different_allocation() {
    let image_program = Arc::new(NonCloneProgram { label: "verified" });
    let entry_program = Arc::new(NonCloneProgram { label: "verified" });
    let exact_owner = owner("build:entry-mismatch");
    assert_eq!(image_program, entry_program);
    assert!(!Arc::ptr_eq(&image_program, &entry_program));

    let image = Arc::new(
        DeploymentImage::try_new(exact_owner.clone(), image_program, [])
            .expect("empty dependency set is valid"),
    );
    let error = PinnedDeploymentEntry::try_new(
        image,
        FakeEntry {
            owner: exact_owner,
            program: entry_program,
            name: "forged-entry",
        },
    )
    .expect_err("value equality cannot substitute for the program Arc");

    assert_eq!(error, PinnedDeploymentEntryError::ProgramMismatch);
}

#[test]
fn entry_rejects_the_same_program_rebound_to_a_different_owner() {
    let program = Arc::new(NonCloneProgram { label: "verified" });
    let image_owner = owner("build:image-owner");
    let entry_owner = owner("build:entry-owner");
    let image = Arc::new(
        DeploymentImage::try_new(image_owner, Arc::clone(&program), [])
            .expect("empty dependency set is valid"),
    );

    let error = PinnedDeploymentEntry::try_new(
        image,
        FakeEntry {
            owner: entry_owner,
            program,
            name: "rebound-entry",
        },
    )
    .expect_err("the same program allocation cannot be rebound to another owner");

    assert_eq!(error, PinnedDeploymentEntryError::OwnerMismatch);
}
