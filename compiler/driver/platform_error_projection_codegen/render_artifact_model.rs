use std::fmt::Write as _;

use super::{
    fingerprint::{FingerprintedCatalog, CODEC_VERSION, REGISTRY_ID, REGISTRY_VERSION},
    rust_names::projection_rust_names,
    PlatformErrorProjectionCodegenError, GENERATED_HEADER,
};

pub(super) fn render_artifact_model(
    catalog: &FingerprintedCatalog<'_>,
) -> Result<String, PlatformErrorProjectionCodegenError> {
    let names = projection_rust_names(catalog)?;
    let mut output = String::new();
    writeln!(output, "{GENERATED_HEADER}").unwrap();
    output.push_str(
        "\nuse std::{fmt, str::FromStr, sync::OnceLock};\n\n\
         use serde::{de::Error as _, Deserialize, Deserializer, Serialize};\n\n",
    );
    writeln!(
        output,
        "pub const PLATFORM_ERROR_PROJECTION_REGISTRY_ID: &str = {REGISTRY_ID:?};"
    )
    .unwrap();
    writeln!(
        output,
        "pub const PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION: u32 = {REGISTRY_VERSION};"
    )
    .unwrap();
    writeln!(
        output,
        "pub const PLATFORM_ERROR_PROJECTION_REGISTRY_FINGERPRINT: &str =\n    {:?};",
        catalog.registry_fingerprint
    )
    .unwrap();
    writeln!(
        output,
        "pub const PLATFORM_ERROR_PROJECTION_CODEC_VERSION: u32 = {CODEC_VERSION};\n"
    )
    .unwrap();

    output.push_str(
        "// Canonical public error symbols end in Error; path-derived Rust variants preserve that suffix.\n\
         #[allow(clippy::enum_variant_names)]\n\
         #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]\n\
         pub enum PlatformErrorProjectionKey {\n",
    );
    for (entry, name) in catalog.entries.iter().zip(&names) {
        writeln!(
            output,
            "    #[serde(rename = {:?})]\n    {name},",
            entry.resolved.projection_key()
        )
        .unwrap();
    }
    output.push_str("}\n\n");

    writeln!(output, "impl PlatformErrorProjectionKey {{").unwrap();
    writeln!(
        output,
        "    pub const ALL: [Self; {}] = [",
        catalog.entries.len()
    )
    .unwrap();
    for name in &names {
        writeln!(output, "        Self::{name},").unwrap();
    }
    output.push_str(
        "    ];\n\n    pub const fn as_str(self) -> &'static str {\n        match self {\n",
    );
    for (entry, name) in catalog.entries.iter().zip(&names) {
        let arm = format!(
            "            Self::{name} => {:?},",
            entry.resolved.projection_key()
        );
        if arm.len() <= 100 {
            writeln!(output, "{arm}").unwrap();
        } else {
            writeln!(output, "            Self::{name} => {{").unwrap();
            writeln!(
                output,
                "                {:?}",
                entry.resolved.projection_key()
            )
            .unwrap();
            output.push_str("            }\n");
        }
    }
    output.push_str("        }\n    }\n\n    pub fn parse_strict(value: &str) -> Result<Self, UnknownPlatformErrorProjectionKey> {\n        value.parse()\n    }\n}\n\n");

    output.push_str(
        "#[derive(Debug, Clone, PartialEq, Eq)]\n\
         pub struct UnknownPlatformErrorProjectionKey {\n\
         \x20   pub value: String,\n\
         }\n\n\
         impl fmt::Display for UnknownPlatformErrorProjectionKey {\n\
         \x20   fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {\n\
         \x20       write!(\n\
         \x20           formatter,\n\
         \x20           \"unknown platform error projection key {:?}\",\n\
         \x20           self.value\n\
         \x20       )\n\
         \x20   }\n\
         }\n\n\
         impl std::error::Error for UnknownPlatformErrorProjectionKey {}\n\n\
         impl FromStr for PlatformErrorProjectionKey {\n\
         \x20   type Err = UnknownPlatformErrorProjectionKey;\n\n\
         \x20   fn from_str(value: &str) -> Result<Self, Self::Err> {\n\
         \x20       match value {\n",
    );
    for (entry, name) in catalog.entries.iter().zip(&names) {
        let arm = format!(
            "            {:?} => Ok(Self::{name}),",
            entry.resolved.projection_key()
        );
        if arm.len() <= 100 {
            writeln!(output, "{arm}").unwrap();
        } else {
            writeln!(
                output,
                "            {:?} => {{",
                entry.resolved.projection_key()
            )
            .unwrap();
            writeln!(output, "                Ok(Self::{name})").unwrap();
            output.push_str("            }\n");
        }
    }
    output.push_str(
        "            _ => Err(UnknownPlatformErrorProjectionKey {\n\
         \x20               value: value.to_owned(),\n\
         \x20           }),\n\
         \x20       }\n\
         \x20   }\n\
         }\n\n\
         impl fmt::Display for PlatformErrorProjectionKey {\n\
         \x20   fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {\n\
         \x20       formatter.write_str(self.as_str())\n\
         \x20   }\n\
         }\n\n",
    );

    output.push_str(
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n\
         pub struct PlatformErrorProjectionDescriptor {\n\
         \x20   key: PlatformErrorProjectionKey,\n\
         \x20   nominal_identity: &'static str,\n\
         \x20   entry_fingerprint: &'static str,\n\
         \x20   codec_version: u32,\n\
         \x20   producer_family: &'static str,\n\
         \x20   semantic_adapter_owner: &'static str,\n\
         \x20   public_message_policy: &'static str,\n\
         \x20   envelope_kind: &'static str,\n\
         \x20   fallback_policy: &'static str,\n\
         }\n\n\
         impl PlatformErrorProjectionDescriptor {\n\
         \x20   pub const fn key(&self) -> PlatformErrorProjectionKey {\n\
         \x20       self.key\n\
         \x20   }\n\n\
         \x20   pub const fn nominal_identity(&self) -> &'static str {\n\
         \x20       self.nominal_identity\n\
         \x20   }\n\n\
         \x20   pub const fn entry_fingerprint(&self) -> &'static str {\n\
         \x20       self.entry_fingerprint\n\
         \x20   }\n\n\
         \x20   pub const fn codec_version(&self) -> u32 {\n\
         \x20       self.codec_version\n\
         \x20   }\n\n\
         \x20   pub const fn producer_family(&self) -> &'static str {\n\
         \x20       self.producer_family\n\
         \x20   }\n\n\
         \x20   pub const fn semantic_adapter_owner(&self) -> &'static str {\n\
         \x20       self.semantic_adapter_owner\n\
         \x20   }\n\n\
         \x20   pub const fn public_message_policy(&self) -> &'static str {\n\
         \x20       self.public_message_policy\n\
         \x20   }\n\n\
         \x20   pub const fn envelope_kind(&self) -> &'static str {\n\
         \x20       self.envelope_kind\n\
         \x20   }\n\n\
         \x20   pub const fn fallback_policy(&self) -> &'static str {\n\
         \x20       self.fallback_policy\n\
         \x20   }\n\
         }\n\n",
    );
    writeln!(
        output,
        "static PLATFORM_ERROR_PROJECTION_REGISTRY: [PlatformErrorProjectionDescriptor; {}] = [",
        catalog.entries.len()
    )
    .unwrap();
    for (entry, name) in catalog.entries.iter().zip(&names) {
        output.push_str("    PlatformErrorProjectionDescriptor {\n");
        writeln!(output, "        key: PlatformErrorProjectionKey::{name},").unwrap();
        writeln!(
            output,
            "        nominal_identity: {:?},",
            entry.resolved.nominal_identity()
        )
        .unwrap();
        writeln!(
            output,
            "        entry_fingerprint:\n            {:?},",
            entry.fingerprint
        )
        .unwrap();
        writeln!(output, "        codec_version: {CODEC_VERSION},").unwrap();
        writeln!(
            output,
            "        producer_family: {:?},",
            entry.resolved.producer_family()
        )
        .unwrap();
        writeln!(
            output,
            "        semantic_adapter_owner: {:?},",
            entry.resolved.semantic_adapter_owner()
        )
        .unwrap();
        writeln!(
            output,
            "        public_message_policy: {:?},",
            entry.resolved.public_message_policy()
        )
        .unwrap();
        writeln!(
            output,
            "        envelope_kind: {:?},",
            entry.resolved.envelope_kind()
        )
        .unwrap();
        writeln!(
            output,
            "        fallback_policy: {:?},",
            entry.resolved.fallback_policy()
        )
        .unwrap();
        output.push_str("    },\n");
    }
    output.push_str(
        "];\n\n\
         pub fn platform_error_projection_registry() -> &'static [PlatformErrorProjectionDescriptor] {\n\
         \x20   &PLATFORM_ERROR_PROJECTION_REGISTRY\n\
         }\n\n\
         pub const fn platform_error_projection_descriptor(\n\
         \x20   key: PlatformErrorProjectionKey,\n\
         ) -> &'static PlatformErrorProjectionDescriptor {\n\
         \x20   match key {\n",
    );
    for (index, name) in names.iter().enumerate() {
        let arm = format!(
            "        PlatformErrorProjectionKey::{name} => &PLATFORM_ERROR_PROJECTION_REGISTRY[{index}],"
        );
        if arm.len() <= 100 {
            writeln!(output, "{arm}").unwrap();
        } else {
            writeln!(output, "        PlatformErrorProjectionKey::{name} => {{").unwrap();
            writeln!(
                output,
                "            &PLATFORM_ERROR_PROJECTION_REGISTRY[{index}]"
            )
            .unwrap();
            output.push_str("        }\n");
        }
    }
    output.push_str(
        "    }\n\
         }\n\n\
         pub fn platform_error_projection_descriptor_by_key(\n\
         \x20   key: &str,\n\
         ) -> Option<&'static PlatformErrorProjectionDescriptor> {\n\
         \x20   PlatformErrorProjectionKey::parse_strict(key)\n\
         \x20       .ok()\n\
         \x20       .map(platform_error_projection_descriptor)\n\
         }\n\n",
    );

    output.push_str(
        "#[derive(Debug, Clone, PartialEq, Eq, Serialize)]\n\
         #[serde(rename_all = \"camelCase\")]\n\
         pub struct PlatformErrorProjectionRegistryRef {\n\
         \x20   registry_id: String,\n\
         \x20   registry_version: u32,\n\
         \x20   fingerprint: String,\n\
         }\n\n\
         #[derive(Deserialize)]\n\
         #[serde(rename_all = \"camelCase\", deny_unknown_fields)]\n\
         struct PlatformErrorProjectionRegistryRefWire {\n\
         \x20   registry_id: String,\n\
         \x20   registry_version: u32,\n\
         \x20   fingerprint: String,\n\
         }\n\n\
         impl PlatformErrorProjectionRegistryRef {\n\
         \x20   pub fn registry_id(&self) -> &str {\n\
         \x20       &self.registry_id\n\
         \x20   }\n\n\
         \x20   pub const fn registry_version(&self) -> u32 {\n\
         \x20       self.registry_version\n\
         \x20   }\n\n\
         \x20   pub fn fingerprint(&self) -> &str {\n\
         \x20       &self.fingerprint\n\
         \x20   }\n\
         }\n\n\
         impl<'de> Deserialize<'de> for PlatformErrorProjectionRegistryRef {\n\
         \x20   fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>\n\
         \x20   where\n\
         \x20       D: Deserializer<'de>,\n\
         \x20   {\n\
         \x20       let wire = PlatformErrorProjectionRegistryRefWire::deserialize(deserializer)?;\n\
         \x20       let descriptor = Self {\n\
         \x20           registry_id: wire.registry_id,\n\
         \x20           registry_version: wire.registry_version,\n\
         \x20           fingerprint: wire.fingerprint,\n\
         \x20       };\n\
         \x20       validate_platform_error_projection_registry_ref_shape(&descriptor)\n\
         \x20           .map_err(|error| D::Error::custom(error.to_string()))?;\n\
         \x20       Ok(descriptor)\n\
         \x20   }\n\
         }\n\n\
         pub fn current_platform_error_projection_registry_ref(\n\
         ) -> &'static PlatformErrorProjectionRegistryRef {\n\
         \x20   static CURRENT: OnceLock<PlatformErrorProjectionRegistryRef> = OnceLock::new();\n\
         \x20   CURRENT.get_or_init(|| PlatformErrorProjectionRegistryRef {\n\
         \x20       registry_id: PLATFORM_ERROR_PROJECTION_REGISTRY_ID.to_owned(),\n\
         \x20       registry_version: PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION,\n\
         \x20       fingerprint: PLATFORM_ERROR_PROJECTION_REGISTRY_FINGERPRINT.to_owned(),\n\
         \x20   })\n\
         }\n\n",
    );

    output.push_str(
        "#[derive(Debug, Clone, PartialEq, Eq)]\n\
         pub enum PlatformErrorProjectionRegistryRefValidationError {\n\
         \x20   RegistryId,\n\
         \x20   RegistryVersion,\n\
         \x20   FingerprintGrammar,\n\
         \x20   CurrentFingerprintMismatch,\n\
         }\n\n\
         impl fmt::Display for PlatformErrorProjectionRegistryRefValidationError {\n\
         \x20   fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {\n\
         \x20       formatter.write_str(match self {\n\
         \x20           Self::RegistryId => \"platform error projection registryId is invalid\",\n\
         \x20           Self::RegistryVersion => \"platform error projection registryVersion is invalid\",\n\
         \x20           Self::FingerprintGrammar => \"platform error projection fingerprint grammar is invalid\",\n\
         \x20           Self::CurrentFingerprintMismatch => {\n\
         \x20               \"platform error projection fingerprint is not current\"\n\
         \x20           }\n\
         \x20       })\n\
         \x20   }\n\
         }\n\n\
         impl std::error::Error for PlatformErrorProjectionRegistryRefValidationError {}\n\n\
         pub fn validate_platform_error_projection_registry_ref_shape(\n\
         \x20   descriptor: &PlatformErrorProjectionRegistryRef,\n\
         ) -> Result<(), PlatformErrorProjectionRegistryRefValidationError> {\n\
         \x20   if descriptor.registry_id() != PLATFORM_ERROR_PROJECTION_REGISTRY_ID {\n\
         \x20       return Err(PlatformErrorProjectionRegistryRefValidationError::RegistryId);\n\
         \x20   }\n\
         \x20   if descriptor.registry_version() != PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION {\n\
         \x20       return Err(PlatformErrorProjectionRegistryRefValidationError::RegistryVersion);\n\
         \x20   }\n\
         \x20   if !is_sha256_fingerprint(descriptor.fingerprint()) {\n\
         \x20       return Err(PlatformErrorProjectionRegistryRefValidationError::FingerprintGrammar);\n\
         \x20   }\n\
         \x20   Ok(())\n\
         }\n\n\
         pub fn validate_current_platform_error_projection_registry_ref(\n\
         \x20   descriptor: &PlatformErrorProjectionRegistryRef,\n\
         ) -> Result<(), PlatformErrorProjectionRegistryRefValidationError> {\n\
         \x20   validate_platform_error_projection_registry_ref_shape(descriptor)?;\n\
         \x20   if descriptor.fingerprint() != PLATFORM_ERROR_PROJECTION_REGISTRY_FINGERPRINT {\n\
         \x20       return Err(PlatformErrorProjectionRegistryRefValidationError::CurrentFingerprintMismatch);\n\
         \x20   }\n\
         \x20   Ok(())\n\
         }\n\n\
         fn is_sha256_fingerprint(value: &str) -> bool {\n\
         \x20   value.strip_prefix(\"sha256:\").is_some_and(|digest| {\n\
         \x20       digest.len() == 64\n\
         \x20           && digest\n\
         \x20               .bytes()\n\
         \x20               .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))\n\
         \x20   })\n\
         }\n\n",
    );

    writeln!(
        output,
        "pub fn assert_platform_error_projection_generated_surface() {{\n    assert_eq!(platform_error_projection_registry().len(), {});",
        catalog.entries.len()
    )
    .unwrap();
    output.push_str(
        "    for (index, descriptor) in platform_error_projection_registry().iter().enumerate() {\n\
         \x20       assert_eq!(descriptor.key(), PlatformErrorProjectionKey::ALL[index]);\n\
         \x20       assert_eq!(descriptor.key().as_str(), descriptor.nominal_identity());\n\
         \x20       let parsed = PlatformErrorProjectionKey::parse_strict(descriptor.key().as_str())\n\
         \x20           .expect(\"generated descriptor key must parse\");\n\
         \x20       assert_eq!(parsed, descriptor.key());\n\
         \x20       assert!(std::ptr::eq(\n\
         \x20           platform_error_projection_descriptor(parsed),\n\
         \x20           descriptor\n\
         \x20       ));\n\
         \x20       assert!(std::ptr::eq(\n\
         \x20           platform_error_projection_descriptor_by_key(descriptor.key().as_str())\n\
         \x20               .expect(\"generated descriptor string key must resolve\"),\n\
         \x20           descriptor\n\
         \x20       ));\n\
         \x20       assert_eq!(\n\
         \x20           descriptor.codec_version(),\n\
         \x20           PLATFORM_ERROR_PROJECTION_CODEC_VERSION\n\
         \x20       );\n\
         \x20       assert!(is_sha256_fingerprint(descriptor.entry_fingerprint()));\n\
         \x20       assert!(!descriptor.producer_family().is_empty());\n\
         \x20       assert!(!descriptor.semantic_adapter_owner().is_empty());\n\
         \x20       assert!(!descriptor.public_message_policy().is_empty());\n\
         \x20       assert!(!descriptor.envelope_kind().is_empty());\n\
         \x20       assert!(!descriptor.fallback_policy().is_empty());\n\
         \x20   }\n\
         \x20   for pair in platform_error_projection_registry().windows(2) {\n\
         \x20       assert!(pair[0].key().as_str() < pair[1].key().as_str());\n\
         \x20   }\n\
         }\n\n\
         #[cfg(test)]\n\
         mod tests {\n\
         \x20   use super::*;\n\n\
         \x20   #[test]\n\
         \x20   fn generated_surface_is_exact_and_unversioned() {\n\
         \x20       assert_platform_error_projection_generated_surface();\n\
         \x20       for key in PlatformErrorProjectionKey::ALL {\n\
         \x20           assert!(!key.as_str().split('.').any(|segment| {\n\
         \x20               segment.strip_prefix('v').is_some_and(|digits| {\n\
         \x20                   !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())\n\
         \x20               })\n\
         \x20           }));\n\
         \x20       }\n\
         \x20   }\n\
         \n\
         \x20   #[test]\n\
         \x20   fn registry_ref_deserialization_validates_general_shape() {\n\
         \x20       let alternate_fingerprint = format!(\"sha256:{}\", \"0\".repeat(64));\n\
         \x20       let alternate: PlatformErrorProjectionRegistryRef =\n\
         \x20           serde_json::from_value(serde_json::json!({\n\
         \x20               \"registryId\": PLATFORM_ERROR_PROJECTION_REGISTRY_ID,\n\
         \x20               \"registryVersion\": PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION,\n\
         \x20               \"fingerprint\": alternate_fingerprint,\n\
         \x20           }))\n\
         \x20           .unwrap();\n\
         \x20       assert_eq!(\n\
         \x20           alternate.registry_id(),\n\
         \x20           PLATFORM_ERROR_PROJECTION_REGISTRY_ID\n\
         \x20       );\n\
         \x20       assert_eq!(\n\
         \x20           alternate.registry_version(),\n\
         \x20           PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION\n\
         \x20       );\n\
         \x20       assert_eq!(\n\
         \x20           alternate.fingerprint(),\n\
         \x20           format!(\"sha256:{}\", \"0\".repeat(64))\n\
         \x20       );\n\
         \x20       assert_eq!(\n\
         \x20           validate_current_platform_error_projection_registry_ref(&alternate),\n\
         \x20           Err(PlatformErrorProjectionRegistryRefValidationError::CurrentFingerprintMismatch)\n\
         \x20       );\n\n\
         \x20       let valid_fingerprint = format!(\"sha256:{}\", \"0\".repeat(64));\n\
         \x20       for invalid in [\n\
         \x20           serde_json::json!({\n\
         \x20               \"registryId\": \"other\",\n\
         \x20               \"registryVersion\": PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION,\n\
         \x20               \"fingerprint\": valid_fingerprint,\n\
         \x20           }),\n\
         \x20           serde_json::json!({\n\
         \x20               \"registryId\": PLATFORM_ERROR_PROJECTION_REGISTRY_ID,\n\
         \x20               \"registryVersion\": 2,\n\
         \x20               \"fingerprint\": valid_fingerprint,\n\
         \x20           }),\n\
         \x20           serde_json::json!({\n\
         \x20               \"registryId\": PLATFORM_ERROR_PROJECTION_REGISTRY_ID,\n\
         \x20               \"registryVersion\": PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION,\n\
         \x20               \"fingerprint\": format!(\"sha256:{}\", \"A\".repeat(64)),\n\
         \x20           }),\n\
         \x20           serde_json::json!({\n\
         \x20               \"registryId\": PLATFORM_ERROR_PROJECTION_REGISTRY_ID,\n\
         \x20               \"registryVersion\": PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION,\n\
         \x20               \"fingerprint\": valid_fingerprint,\n\
         \x20               \"unexpected\": true,\n\
         \x20           }),\n\
         \x20       ] {\n\
         \x20           assert!(serde_json::from_value::<PlatformErrorProjectionRegistryRef>(invalid).is_err());\n\
         \x20       }\n\
         \x20   }\n\n\
         \x20   #[test]\n\
         \x20   fn current_registry_ref_is_one_exact_read_only_singleton() {\n\
         \x20       let current = current_platform_error_projection_registry_ref();\n\
         \x20       assert!(std::ptr::eq(\n\
         \x20           current,\n\
         \x20           current_platform_error_projection_registry_ref()\n\
         \x20       ));\n\
         \x20       assert_eq!(current.registry_id(), PLATFORM_ERROR_PROJECTION_REGISTRY_ID);\n\
         \x20       assert_eq!(\n\
         \x20           current.registry_version(),\n\
         \x20           PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION\n\
         \x20       );\n\
         \x20       assert_eq!(\n\
         \x20           current.fingerprint(),\n\
         \x20           PLATFORM_ERROR_PROJECTION_REGISTRY_FINGERPRINT\n\
         \x20       );\n\
         \x20       validate_current_platform_error_projection_registry_ref(current).unwrap();\n\
         \x20       assert_eq!(\n\
         \x20           serde_json::to_value(current).unwrap(),\n\
         \x20           serde_json::json!({\n\
         \x20               \"registryId\": PLATFORM_ERROR_PROJECTION_REGISTRY_ID,\n\
         \x20               \"registryVersion\": PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION,\n\
         \x20               \"fingerprint\": PLATFORM_ERROR_PROJECTION_REGISTRY_FINGERPRINT,\n\
         \x20           })\n\
         \x20       );\n\
         \x20   }\n\
         }\n",
    );
    Ok(output)
}
