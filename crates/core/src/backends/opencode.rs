//! opencode-family backends: `opencode` / `kilocode` / `mimocode` share one implementation for three sibling formats.
//! The top-level `agent` (subagent definition map), `provider` (map), and other top-level fields (e.g. `mcp`) are
//! preserved verbatim. The three differ only in config directory, primary file name, `$schema`, and icon,
//! distinguished by [`Flavor`].

use super::{Backend, BackendLoad};
use crate::convert;
use crate::format::ConfigFormat;
use crate::model::{AgentRow, OpencodeSchemaVersion, ProviderRow};
use crate::util::{home_dir_string, parse_config_content, wsl_home, WslPathProbe};
use serde_json::{Map, Value};
use std::path::Path;

/// One concrete member of the opencode family: only the directory, file name, `$schema`, and icon differ.
pub struct Flavor {
    pub id: ConfigFormat,
    /// The config directory name (`~/.config/<dir>/`).
    pub dir: &'static str,
    /// The primary config file name; Kilo's and MiMo's `.jsonc` variants are accepted too.
    pub file: &'static str,
    /// The official `$schema` address: written on create / merge when missing; existing values always kept.
    pub schema: &'static str,
    /// 32×32 non-premultiplied RGBA raw bytes.
    pub icon: &'static [u8],
}

pub struct OpenCodeFamilyBackend(pub &'static Flavor);

/// opencode itself.
pub static OPENCODE: Flavor = Flavor {
    id: ConfigFormat::Opencode,
    dir: "opencode",
    file: "opencode.json",
    schema: "https://opencode.ai/config.json",
    icon: include_bytes!("../../assets/agents/opencode_32.bin"),
};

/// Kilo Code (`@kilocode/cli`): config dir `~/.config/kilo/`, primary config `kilo.json`;
/// the leftover `opencode.json` in the same directory is also read, but the `.opencode` directory is not a fallback.
pub static KILOCODE: Flavor = Flavor {
    id: ConfigFormat::Kilocode,
    dir: "kilo",
    file: "kilo.json",
    schema: "https://app.kilo.ai/config.json",
    icon: include_bytes!("../../assets/agents/kilocode_32.bin"),
};

/// MiMo Code (`@mimo-ai/cli`, Xiaomi): config dir `~/.config/mimocode/` (Windows:
/// `%LOCALAPPDATA%\mimocode\`), primary config `mimocode.json`; `opencode.json` is not read.
/// Icon is Xiaomi's official logo (orange with a white `mi`).
pub static MIMOCODE: Flavor = Flavor {
    id: ConfigFormat::Mimocode,
    dir: "mimocode",
    file: "mimocode.json",
    schema: "https://mimo.xiaomi.com/mimocode/config.json",
    icon: include_bytes!("../../assets/agents/mimocode_32.bin"),
};

/// The opencode backend instance.
pub static BACKEND: OpenCodeFamilyBackend = OpenCodeFamilyBackend(&OPENCODE);

/// The Kilo Code backend instance.
pub static KILOCODE_BACKEND: OpenCodeFamilyBackend = OpenCodeFamilyBackend(&KILOCODE);

/// The MiMo Code backend instance.
pub static MIMOCODE_BACKEND: OpenCodeFamilyBackend = OpenCodeFamilyBackend(&MIMOCODE);

impl Flavor {
    fn default_local_path(&self) -> String {
        format!(
            "{}\\.config\\{}\\{}",
            home_dir_string(),
            self.dir,
            self.file
        )
    }

    fn default_wsl_path(&self) -> Option<String> {
        Some(format!(
            "{}/.config/{}/{}",
            wsl_home()?,
            self.dir,
            self.file
        ))
    }

    /// The primary config's `.jsonc` variant path (`kilo.json` → `kilo.jsonc`).
    fn jsonc_variant(&self, path: &str) -> String {
        let stem = self.file.trim_end_matches(".json");
        // Only the file-name segment is replaced.
        match path.rfind(&self.file) {
            Some(at) => format!("{}{}.jsonc", &path[..at], stem),
            // The primary file name isn't in the path: swap by extension wholesale.
            None => format!("{}.jsonc", path.trim_end_matches(".json")),
        }
    }
}

impl Backend for OpenCodeFamilyBackend {
    fn id(&self) -> ConfigFormat {
        self.0.id
    }

    fn default_local_path(&self) -> String {
        self.0.default_local_path()
    }

    fn default_wsl_path(&self) -> Option<String> {
        self.0.default_wsl_path()
    }

    /// Fall back to the `.jsonc` variant when the default name is missing; candidates are idempotent — an already-`.jsonc` input isn't appended again.
    fn path_candidates(&self, path: &str) -> Vec<String> {
        let variant = self.0.jsonc_variant(path);
        if variant == path {
            vec![path.to_string()]
        } else {
            vec![path.to_string(), variant]
        }
    }

    fn local_available(&self, local_path: &str) -> bool {
        self.path_candidates(local_path)
            .iter()
            .any(|p| Path::new(p.as_str()).exists())
    }

    fn wsl_available(&self, probe: WslPathProbe) -> bool {
        // Installed verdict: the config file or its directory exists
        probe.file_exists || probe.parent_dir_exists
    }

    /// Detection: claim by path first (directory or file name), then fall back to content fingerprints.
    ///
    /// The content fallback applies only to opencode itself, and only when no family member claimed by path.
    fn detect(&self, content: &str, path: &str) -> bool {
        if path_matches(self.0, path) {
            return true;
        }
        // Any family member already claimed by path; the content fallback stands down.
        if FAMILY.iter().any(|f| path_matches(f, path)) {
            return false;
        }
        if self.0.id != ConfigFormat::Opencode {
            return false;
        }
        // Positive fingerprint: the top level holds a provider object (both singular and plural keys).
        parse_config_content(content)
            .map(|v| providers_table(&v).is_some())
            .unwrap_or(false)
    }

    fn parse(&self, content: &str) -> Result<BackendLoad, String> {
        let v = parse_config_content(content)?;
        // An entry's schema version follows the **file**: a v2 file's minimal agent (no request, model without #)
        // would be misjudged as v1 by per-entry sniffing — temperature read from the top-level dead key, and
        // request.body.temperature silently deleted at save.
        let file_schema = detect_schema_version(&v);
        let agents = agents_table(&v)
            .map(|o| {
                o.iter()
                    .map(|(k, av)| AgentRow::from_with_schema(k, av, file_schema))
                    .collect()
            })
            .unwrap_or_default();
        let providers = providers_table(&v)
            .map(|o| o.iter().map(|(k, pv)| ProviderRow::from(k, pv)).collect())
            .unwrap_or_default();
        // The row's schema records raw's real shape (for same-version incremental writes); "kilo / mimo always v1"
        // is enforced by the write path's target_version clamp (see serialize_root) — not by rewriting the row's
        // schema here, or same-version writes would keep raw's v2 keys.
        Ok(BackendLoad {
            root: v.clone(),
            agents,
            providers,
            // The extras carrier IS the whole root (agent/provider saves replace wholesale)
            extras: v,
        })
    }

    fn serialize_root(
        &self,
        agents: &[AgentRow],
        providers: &[ProviderRow],
        extras: &Value,
        target_root: Option<&Value>,
    ) -> Value {
        let schema = self.0.schema;
        let mut root = match target_root {
            None => {
                // Current file: agent / provider replaced wholesale per UI state (deletions take effect); an empty list
                // removes the corresponding key rather than writing an empty object
                let mut r = extras.clone();
                // Key names and write-back shape follow the source file: plural keys present → write plural, else singular;
                // v2 files write the v2 shape (package / settings / capabilities etc.). Taken before the mutable borrow
                // to avoid borrow conflicts.
                let ak = agents_key(&r);
                let pk = providers_key(&r);
                let version = target_version(self.0.id, &r);
                if let Value::Object(o) = &mut r {
                    let mut am = Map::new();
                    for a in agents {
                        if !a.key.is_empty() {
                            am.insert(a.key.clone(), a.to_value_as(version));
                        }
                    }
                    if am.is_empty() {
                        o.remove(ak);
                    } else {
                        o.insert(ak.into(), Value::Object(am));
                    }

                    let mut pm = Map::new();
                    for p in providers {
                        if !p.key.is_empty() {
                            pm.insert(p.key.clone(), p.to_value_as(version));
                        }
                    }
                    if pm.is_empty() {
                        o.remove(pk);
                    } else {
                        o.insert(pk.into(), Value::Object(pm));
                    }
                }
                with_schema(r, schema)
            }
            Some(target) => with_schema(
                merge_opencode_root_as(
                    target,
                    agents,
                    providers,
                    target_version(self.0.id, target),
                ),
                schema,
            ),
        };
        // Complete required fields per the target dialect, including old entries kept in the target file.
        complete_required_fields_in(&mut root, self.0.id);
        root
    }

    fn load_target_root(&self, path: &str) -> Result<Value, String> {
        super::load_target_root_with(path, parse_config_content, || Value::Object(Map::new()))
    }

    fn icon_rgba(&self) -> Option<(&'static [u8], u32, u32)> {
        Some((self.0.icon, 32, 32))
    }
}

/// Every opencode-family member.
pub static FAMILY: &[&Flavor] = &[&OPENCODE, &KILOCODE, &MIMOCODE];

/// Whether a path belongs to any opencode-family member.
///
/// Lets other backends' content-fallback detection stand down: newer opencode writes plural `providers`
/// (no `provider`) at the top level, overlapping the pi family's content fingerprints — content alone
/// would wrongly claim it.
pub fn family_path_matches(path: &str) -> bool {
    FAMILY.iter().any(|f| path_matches(f, path))
}

/// Whether a path belongs to a given opencode-family member: its directory or file name appears in the path segments.
///
/// Accepts both `/` and `\` separators, case-insensitively.
fn path_matches(flavor: &Flavor, path: &str) -> bool {
    if path.trim().is_empty() {
        return false;
    }
    let lower = path.to_lowercase().replace('\\', "/");
    // A directory-name hit claims it.
    let dir_seg = format!("/{}/", flavor.dir);
    if lower.contains(&dir_seg) {
        return true;
    }
    // When another family member's directory name appears, the file name is no longer this member's clue.
    let other_dir = FAMILY
        .iter()
        .any(|f| f.dir != flavor.dir && lower.contains(&format!("/{}/", f.dir)));
    if other_dir {
        return false;
    }
    // File name: the primary and its `.jsonc` variant.
    let stem = flavor.file.trim_end_matches(".json");
    let names = [format!("/{}", flavor.file), format!("/{}.jsonc", stem)];
    names.iter().any(|n| lower.ends_with(n.as_str()))
}

/// Ensure `$schema` exists and comes first; existing values are always kept.
fn with_schema(root: Value, schema: &str) -> Value {
    let Value::Object(mut o) = root else {
        return root;
    };
    if !o.contains_key("$schema") {
        o.insert("$schema".into(), Value::String(schema.to_string()));
    }
    Value::Object(convert::order_fields(o, &["$schema"]))
}

/// Complete model-level **required pair fields** per the target dialect.
///
/// The three CLIs differ in requirements: `limit.context` / `limit.output` must be paired by all three;
/// modality pairing (v1 in `modalities`, v2 in `capabilities`) is required only by mimocode.
///
/// Completion: the missing side gets `["text"]`; a half `limit` is removed whole (`limit` is optional at
/// model level). When neither modality side is written: v1 removes the block whole (`{}` is invalid);
/// v2 completes both sides if keys like `tools` remain, else removes it.
fn complete_required_model_fields(
    model: &mut Value,
    flavor: ConfigFormat,
    version: OpencodeSchemaVersion,
) {
    let Some(obj) = model.as_object_mut() else {
        return;
    };

    // limit: all three require context + output paired (the block may be omitted whole, never half-given).
    let limit_sides = obj
        .get("limit")
        .and_then(Value::as_object)
        .map(|l| (l.contains_key("context"), l.contains_key("output")));
    if let Some((has_context, has_output)) = limit_sides {
        if has_context != has_output {
            obj.remove("limit");
        }
    }

    // Modality pairing: only mimocode requires it.
    if flavor != ConfigFormat::Mimocode {
        return;
    }
    let parent = match version {
        OpencodeSchemaVersion::V1 => "modalities",
        OpencodeSchemaVersion::V2 => "capabilities",
    };
    let modality_sides = obj
        .get(parent)
        .and_then(Value::as_object)
        .map(|m| (m.contains_key("input"), m.contains_key("output")));
    let Some((has_input, has_output)) = modality_sides else {
        return;
    };
    match (has_input, has_output) {
        (true, true) => {}
        (false, false) => {
            let has_other = obj
                .get(parent)
                .and_then(Value::as_object)
                .is_some_and(|m| !m.is_empty());
            if has_other {
                fill_text_modality(obj, parent, "input");
                fill_text_modality(obj, parent, "output");
            } else {
                obj.remove(parent);
            }
        }
        (true, false) => fill_text_modality(obj, parent, "output"),
        (false, true) => fill_text_modality(obj, parent, "input"),
    }
}

/// Complete `<parent>.<side>` to `["text"]`.
fn fill_text_modality(obj: &mut Map<String, Value>, parent: &str, side: &str) {
    if let Some(target) = obj.get_mut(parent).and_then(Value::as_object_mut) {
        target.insert(
            side.into(),
            Value::Array(vec![Value::String("text".into())]),
        );
    }
}

/// Complete required fields for every entry of the root's provider table, including models already in the target file.
/// Handles old and new top-level keys: newer files write `providers`, older ones `provider`.
fn complete_required_fields_in(root: &mut Value, flavor: ConfigFormat) {
    let version = detect_schema_version(root);
    let pk = providers_key(root);
    let Some(providers) = root.get_mut(pk).and_then(Value::as_object_mut) else {
        return;
    };
    for provider in providers.values_mut() {
        let Some(models) = provider.get_mut("models").and_then(Value::as_object_mut) else {
            continue;
        };
        for model in models.values_mut() {
            complete_required_model_fields(model, flavor, version);
        }
    }
}

/// Merge UI state into an opencode-family target root (for cross-format saves):
/// agent / provider entries are upserted from UI state; same-named target entries merge conservatively
/// by field (UI-provided keys win; target-only keys and target-only entries are always kept); other
/// top-level fields are preserved verbatim.
/// Top-level agents / providers key names follow the source file: newer opencode writes plural
/// `agents` / `providers`, older versions singular `agent` / `provider`.
/// Plural wins when present, else singular.
fn agents_key(root: &Value) -> &'static str {
    if root.get("agents").and_then(|x| x.as_object()).is_some() {
        "agents"
    } else {
        "agent"
    }
}

fn providers_key(root: &Value) -> &'static str {
    // opencode's conventional key is the singular `provider`: when the target holds singular and plural side by
    // side (the deliberate case of keeping pi's leftover top-level `providers`), the singular must still be
    // written, or provider entries would be upserted into the pi-shaped container. Plural is followed only
    // when the target has plural keys alone.
    if root.get("provider").and_then(|x| x.as_object()).is_none()
        && root.get("providers").and_then(|x| x.as_object()).is_some()
    {
        "providers"
    } else {
        "provider"
    }
}

fn agents_table(root: &Value) -> Option<&Map<String, Value>> {
    root.get(agents_key(root)).and_then(|x| x.as_object())
}

fn providers_table(root: &Value) -> Option<&Map<String, Value>> {
    root.get(providers_key(root)).and_then(|x| x.as_object())
}

/// Detect the opencode schema version of a whole file.
///
/// Provider using `package` / `settings` is the strongest v2 evidence; top-level plural `agents` /
/// `plugins` (pi never has them; old opencode writes singular) and an agent's `model#variant` are
/// backups. Top-level plural `providers` **alone is not evidence**: that is also pi's content shape
/// (its top level is providers), and bare plural keys belong to pi.
/// Write-back shape follows this verdict: given v1, update v1; given v2, update v2.
pub fn detect_schema_version(root: &Value) -> OpencodeSchemaVersion {
    if providers_table(root).is_some_and(|table| {
        table
            .values()
            .any(|p| p.get("package").is_some() || p.get("settings").is_some())
    }) {
        return OpencodeSchemaVersion::V2;
    }
    if root.get("agents").and_then(Value::as_object).is_some() || root.get("plugins").is_some() {
        return OpencodeSchemaVersion::V2;
    }
    if agents_table(root).is_some_and(|table| {
        table
            .values()
            .any(|a| crate::util::str_at(a, "model").contains('#'))
    }) {
        return OpencodeSchemaVersion::V2;
    }
    OpencodeSchemaVersion::V1
}

/// Read/write version of an opencode-family backend: opencode follows file content (v1 or v2);
/// kilo / mimo are always v1 — v2 is opencode's own evolution and their CLIs never adopted the v2
/// schema (user decision).
fn target_version(id: ConfigFormat, root: &Value) -> OpencodeSchemaVersion {
    if id == ConfigFormat::Opencode {
        detect_schema_version(root)
    } else {
        OpencodeSchemaVersion::V1
    }
}

pub fn merge_opencode_root(
    target_root: &Value,
    agents: &[AgentRow],
    providers: &[ProviderRow],
) -> Value {
    merge_opencode_root_as(
        target_root,
        agents,
        providers,
        detect_schema_version(target_root),
    )
}

/// Same as [`merge_opencode_root`], but the caller specifies the write-back version
/// (the opencode-family save path uses it to pin kilo / mimo to always write v1).
pub fn merge_opencode_root_as(
    target_root: &Value,
    agents: &[AgentRow],
    providers: &[ProviderRow],
    version: OpencodeSchemaVersion,
) -> Value {
    // Key names and write-back shape follow the target file: plural keys present → plural, else singular;
    // a v2 target writes the v2 shape. Taken before the mutable borrow to avoid borrow conflicts.
    let ak = agents_key(target_root);
    let pk = providers_key(target_root);
    let mut root = target_root.clone();
    if let Value::Object(o) = &mut root {
        let mut am = o
            .get(ak)
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        for a in agents {
            if !a.key.is_empty() {
                let value = a.to_value_as(version);
                let entry = match am.get(&a.key) {
                    Some(target) => convert::merge_conservative(target, &value),
                    None => value,
                };
                am.insert(a.key.clone(), entry);
            }
        }
        o.insert(ak.into(), Value::Object(am));

        let mut pm = o
            .get(pk)
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        for p in providers {
            if !p.key.is_empty() {
                let value = p.to_value_as(version);
                let entry = match pm.get(&p.key) {
                    Some(target) => convert::merge_conservative(target, &value),
                    None => value,
                };
                pm.insert(p.key.clone(), entry);
            }
        }
        o.insert(pk.into(), Value::Object(pm));
    }
    root
}
