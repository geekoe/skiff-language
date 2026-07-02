use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn is_test_file_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".test.skiff"))
}

pub fn production_relative_path_for_test_file(path: &Path) -> Option<PathBuf> {
    let FriendProductionMatch::Unique(path) = production_friend_match_for_test_file(path).ok()?
    else {
        return None;
    };
    Some(path)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FriendProductionMatch {
    None,
    Unique(PathBuf),
    Ambiguous(Vec<PathBuf>),
}

pub fn production_friend_match_for_test_file(
    path: &Path,
) -> std::io::Result<FriendProductionMatch> {
    if !is_test_file_path(path) {
        return Ok(FriendProductionMatch::None);
    }
    let Some(parent) = path.parent() else {
        return Ok(FriendProductionMatch::None);
    };
    let Some(test_base) = test_file_basename(path) else {
        return Ok(FriendProductionMatch::None);
    };
    let mut candidates = Vec::new();
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let candidate = entry.path();
        if !candidate.is_file() || is_test_file_path(&candidate) {
            continue;
        }
        let Some(production_base) = production_file_basename(&candidate) else {
            continue;
        };
        if test_base == production_base || test_base.starts_with(&format!("{production_base}.")) {
            candidates.push(candidate);
        }
    }
    candidates.sort();
    Ok(match candidates.len() {
        0 => FriendProductionMatch::None,
        1 => FriendProductionMatch::Unique(candidates.remove(0)),
        _ => FriendProductionMatch::Ambiguous(candidates),
    })
}

pub fn production_file_basename(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_str()?;
    if is_test_file_path(path) {
        return None;
    }
    file_name.strip_suffix(".skiff").map(str::to_string)
}

pub fn test_file_basename(path: &Path) -> Option<String> {
    path.file_name()?
        .to_str()?
        .strip_suffix(".test.skiff")
        .map(str::to_string)
}

pub fn module_relative_path_for_test_file_without_friend(path: &Path) -> PathBuf {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return path.to_path_buf();
    };
    let Some(test_base) = file_name.strip_suffix(".test.skiff") else {
        return path.to_path_buf();
    };
    path.with_file_name(format!("{test_base}.skiff"))
}

pub fn is_friend_test_file_for_production(test_path: &Path, production_path: &Path) -> bool {
    if test_path.parent() != production_path.parent() {
        return false;
    }
    let Some(test_base) = test_file_basename(test_path) else {
        return false;
    };
    let Some(production_base) = production_file_basename(production_path) else {
        return false;
    };
    test_base == production_base || test_base.starts_with(&format!("{production_base}."))
}
