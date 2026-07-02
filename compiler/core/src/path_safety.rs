use crate::id::PublicationId;

pub fn is_safe_publication_artifact_path_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.trim() == segment
        && segment != "."
        && segment != ".."
        && !segment.contains(['/', '\\'])
        && !segment.chars().any(char::is_whitespace)
}

pub fn is_safe_publication_artifact_id_component(id: &str) -> bool {
    PublicationId::parse(id)
        .map(|id| is_safe_publication_artifact_path_segment(&id.artifact_path()))
        .unwrap_or(false)
}
