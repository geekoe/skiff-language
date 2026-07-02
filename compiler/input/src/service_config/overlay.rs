use serde_yaml::{Mapping as YamlMapping, Value as YamlValue};

pub(super) fn overlay_yaml_object(target: &mut YamlValue, overlay: YamlValue) {
    let YamlValue::Mapping(target_mapping) = target else {
        return;
    };
    let YamlValue::Mapping(overlay_mapping) = overlay else {
        return;
    };
    overlay_yaml_mapping(target_mapping, overlay_mapping);
}

fn overlay_yaml_mapping(target: &mut YamlMapping, overlay: YamlMapping) {
    for (key, value) in overlay {
        match value {
            YamlValue::Null => {
                target.remove(&key);
            }
            YamlValue::Mapping(overlay_child) => match target.get_mut(&key) {
                Some(YamlValue::Mapping(target_child)) => {
                    overlay_yaml_mapping(target_child, overlay_child);
                }
                _ => {
                    target.insert(key, YamlValue::Mapping(overlay_child));
                }
            },
            other => {
                target.insert(key, other);
            }
        }
    }
}
