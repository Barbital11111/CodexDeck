use std::collections::BTreeSet;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine;
use flate2::read::GzDecoder;
use regex::Regex;
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const HISTORY_PROVIDER_FILTER_BEFORE: &str = "modelProviders:null";
const HISTORY_PROVIDER_FILTER_AFTER: &str = "modelProviders:[]  ";
const STABLE_METADATA_NON_RETRY_BEFORE: &str = "return(e instanceof Error?e.message:String(e)).toLowerCase().includes(`you have not agreed to the xcode license agreements`)";
const STABLE_METADATA_NON_RETRY_AFTER: &str = "return/you have not agreed to the xcode license agreements|not a git repository/i.test(e instanceof Error?e.message:e)";

const LEGACY_SOL_POWER_SELECTIONS_PATCH_MARKER: &str = "/*codexdeck-sol-max*/";
const PREVIOUS_POWER_SELECTIONS_PATCH_MARKERS: [&str; 3] = [
    "/*codexdeck-power-v2*/",
    "/*codexdeck-power-v3*/",
    "/*codexdeck-power-v4*/",
];
const POWER_SELECTIONS_PATCH_MARKER: &str = "/*codexdeck-power-v5*/";
const LEGACY_SOL_POWER_SELECTIONS: &str = concat!(
    "{id:`gpt-5.6-terra:low`,model:`gpt-5.6-terra`,modelLabel:`5.6 Terra`,reasoningEffort:`low`},",
    "{id:`gpt-5.6-sol:low`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`low`},",
    "{id:`gpt-5.6-sol:medium`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`medium`},",
    "{id:`gpt-5.6-sol:high`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`high`},",
    "{id:`gpt-5.6-sol:xhigh`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`xhigh`},",
    "{id:`gpt-5.6-sol:ultra`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`ultra`}",
);
const LEGACY_SPLIT_POWER_SELECTIONS: &str = concat!(
    "{id:`gpt-5.6-terra:low`,model:`gpt-5.6-terra`,modelLabel:`5.6 Terra`,reasoningEffort:`low`},",
    "{id:`gpt-5.6-sol:low`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`low`},",
    "{id:`gpt-5.6-sol:medium`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`medium`},",
    "{id:`gpt-5.6-sol:high`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`high`},",
    "{id:`gpt-5.6-sol:xhigh`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`xhigh`}",
);
const MANAGED_POWER_SELECTIONS_EXPRESSION: &str = concat!(
    "`terra,sol,luna`.split`,`",
    ".flatMap(t=>`low,medium,high,xhigh,max,ultra`.split`,`",
    ".map(e=>({id:`gpt-5.6-${t}:${e}`,model:`gpt-5.6-${t}`,",
    "modelLabel:`5.6 ${t[0].toUpperCase()+t.slice(1)}`,reasoningEffort:t===`luna`&&e===`ultra`?`max`:e})))"
);
const LEGACY_POWER_SELECTION_FILTER: &str = "function K(e){return q.flatMap((t,n)=>e?.some(e=>e.model===t.model&&e.supportedReasoningEfforts.some(({reasoningEffort:e})=>e===t.reasoningEffort))?[{...t,powerSettingIndex:n}]:[])}";
const LEGACY_FALLBACK_POWER_SELECTION_FILTER: &str =
    "function K(e){let t=q(ce,e);if(t.length>=4)return t;let n=q(le,e);return n.length>=4?n:[]}";
const LEGACY_SPLIT_POWER_SELECTION_FILTER: &str =
    "function ae(e,t=!1){let n=K(t?[...q,le]:q,e);if(n.length>=4)return n;let r=K(ue,e);return r.length>=4?r:[]}";
const CURRENT_LEGACY_POWER_SELECTION_FILTER: &str =
    "function K(e){return q.filter(t=>t.modelLabel=e?.find(e=>e.model==t.model)?.displayName)}";
const CURRENT_FALLBACK_POWER_SELECTION_FILTER: &str =
    "function K(e){return ce.filter(t=>t.modelLabel=e?.find(e=>e.model==t.model)?.displayName)}";
const CURRENT_SPLIT_POWER_SELECTION_FILTER: &str =
    "function ae(e){return q.filter(t=>t.modelLabel=e?.find(e=>e.model==t.model)?.displayName)}";
const PREVIOUS_V5_LEGACY_POWER_SELECTION_FILTER: &str =
    "function K(e){return q.filter(t=>e?.some(e=>e.model===t.model))}";
const PREVIOUS_V5_FALLBACK_POWER_SELECTION_FILTER: &str =
    "function K(e){return ce.filter(t=>e?.some(e=>e.model===t.model))}";
const PREVIOUS_CURRENT_POWER_SELECTION_FILTER: &str =
    "function K(e,t){return oe(e?.filter(e=>e.model===t))}";
const POWER_SELECTIONS_EXPORT_NAME: &str = "h";
const COLLAPSED_REASONING_LABELS_JSON: &str = r#"{"low":"Low","medium":"Medium","high":"High","xhigh":"Extra high","max":"Max","ultra":"ULTRA"}"#;

const ZH_CN_MAX_LABEL_BEFORE: &str = "\"composer.mode.local.reasoning.max.label\":`最高`";
const ZH_CN_MAX_LABEL_AFTER: &str = "\"composer.mode.local.reasoning.max.label\":`Max`";
const ZH_CN_ULTRA_LABEL_BEFORE: &str = "\"composer.mode.local.reasoning.ultra.label\":`极高`";
const ZH_CN_ULTRA_LABEL_AFTER: &str = "\"composer.mode.local.reasoning.ultra.label\":`超高`";
const REASONING_LABEL_DEFAULT_REPLACEMENTS: [(&str, &str); 3] = [
    ("defaultMessage:`Light`", "defaultMessage:`Low`"),
    ("defaultMessage:`Extra High`", "defaultMessage:`Extra high`"),
    ("defaultMessage:`Ultra`", "defaultMessage:`ULTRA`"),
];

const CUSTOM_PICKER_UI_MARKER: &str = "codexdeck-model-picker-ui-v7";
const CUSTOM_PICKER_CSS_MARKER: &str = ".cdpv8";
const DEFAULT_PATCH_VERSION: &str = "model-picker-v23";
const CUSTOM_PICKER_COMPONENT_SHA256: &str =
    "4172a556d950840debd0b40f0c652a59e030fb7720f76d0d6d8c3392f4d90faa";
const CUSTOM_PICKER_CSS_SHA256: &str =
    "d8a366bf2ebb7cfeccf479de46cacb5ccedce36324c6e95552dc2ad7837bc2dc";
const ORIGINAL_POWER_SLIDER_FUNCTION_SHA256: &str =
    "7a560ef9b0622ba49133a9f354787cef3eea2b773839b4122cf7dc67c0886bd7";
const ORIGINAL_POWER_SLIDER_CSS_SHA256: &str =
    "8471822c996dbfa4c600439679d99a77ab27ee37e9aee0fc413c326ab1c18fdb";

// These are the compressed JS/CSS payloads from the former Node patcher. They are
// embedded in the binary, so installed builds never need Node or the script file.
const CUSTOM_PICKER_COMPONENT_GZIP_BASE64: &str = "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";
const CUSTOM_PICKER_CSS_GZIP_BASE64: &str = "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";

const IDENT: &str = r"[A-Za-z_$][A-Za-z0-9_$]*";
const INTEGRITY_DEFAULT_BLOCK_SIZE: usize = 4_194_304;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub(crate) struct PatchRequest {
    pub(crate) app_asar_path: PathBuf,
    pub(crate) backup_dir: PathBuf,
    pub(crate) patch_state_path: PathBuf,
    pub(crate) patch_version: String,
    pub(crate) source_codex_version: Option<String>,
    pub(crate) source_asar_hash: Option<String>,
    pub(crate) controlled_app_root: Option<PathBuf>,
    pub(crate) launch_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum PatchDisposition {
    Patchable,
    Patched,
    Missing,
    Ambiguous,
    UnsafeSizeChange,
    UnsupportedPreimage,
}

impl PatchDisposition {
    fn is_failure(self) -> bool {
        matches!(
            self,
            Self::Missing | Self::Ambiguous | Self::UnsafeSizeChange | Self::UnsupportedPreimage
        )
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Patchable => "patchable",
            Self::Patched => "patched",
            Self::Missing => "missing",
            Self::Ambiguous => "ambiguous",
            Self::UnsafeSizeChange => "unsafe-size-change",
            Self::UnsupportedPreimage => "unsupported-preimage",
        }
    }
}

#[derive(Debug)]
struct TextPatch {
    state: PatchDisposition,
    text: Option<String>,
}

impl TextPatch {
    fn state(state: PatchDisposition) -> Self {
        Self { state, text: None }
    }

    fn patchable(text: String) -> Self {
        Self {
            state: PatchDisposition::Patchable,
            text: Some(text),
        }
    }

    fn patched() -> Self {
        Self::state(PatchDisposition::Patched)
    }
}

#[derive(Debug)]
struct EntryPatch {
    state: PatchDisposition,
    bytes: Option<Vec<u8>>,
    patch_name: Option<&'static str>,
    additional_patch_names: &'static [&'static str],
}

impl EntryPatch {
    fn state(state: PatchDisposition) -> Self {
        Self {
            state,
            bytes: None,
            patch_name: None,
            additional_patch_names: &[],
        }
    }

    fn patched(patch_name: &'static str) -> Self {
        Self {
            state: PatchDisposition::Patched,
            bytes: None,
            patch_name: Some(patch_name),
            additional_patch_names: &[],
        }
    }

    fn patchable(bytes: Vec<u8>, patch_name: &'static str) -> Self {
        Self {
            state: PatchDisposition::Patchable,
            bytes: Some(bytes),
            patch_name: Some(patch_name),
            additional_patch_names: &[],
        }
    }

    fn with_additional_patch_names(mut self, patch_names: &'static [&'static str]) -> Self {
        self.additional_patch_names = patch_names;
        self
    }
}

#[derive(Debug, Clone)]
struct AsarFile {
    path: String,
    entry_path: Vec<String>,
    entry: Value,
    offset: u64,
    size: usize,
}

#[derive(Debug)]
struct AsarHeader {
    header_bytes: Vec<u8>,
    header: Value,
    ordered_header: OrderedJsonValue,
    files_offset: u64,
}

// serde_json::Value uses a BTreeMap unless the crate-wide preserve_order feature
// is enabled. ASAR headers must retain their original object-member order when
// rewritten in place, so keep a small ordered representation alongside Value.
#[derive(Debug, Clone)]
enum OrderedJsonValue {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<OrderedJsonValue>),
    Object(Vec<(String, OrderedJsonValue)>),
}

impl<'de> Deserialize<'de> for OrderedJsonValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct OrderedJsonVisitor;

        impl<'de> Visitor<'de> for OrderedJsonVisitor {
            type Value = OrderedJsonValue;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON value")
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(OrderedJsonValue::Bool(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(OrderedJsonValue::Number(value.into()))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(OrderedJsonValue::Number(value.into()))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                serde_json::Number::from_f64(value)
                    .map(OrderedJsonValue::Number)
                    .ok_or_else(|| E::custom("JSON number must be finite"))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(OrderedJsonValue::String(value.to_string()))
            }

            fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(OrderedJsonValue::String(value.to_string()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(OrderedJsonValue::String(value))
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(OrderedJsonValue::Null)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(OrderedJsonValue::Null)
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element()? {
                    values.push(value);
                }
                Ok(OrderedJsonValue::Array(values))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut fields = Vec::<(String, OrderedJsonValue)>::new();
                while let Some((key, value)) = map.next_entry::<String, OrderedJsonValue>()? {
                    if let Some((_, existing)) = fields.iter_mut().find(|(name, _)| name == &key) {
                        *existing = value;
                    } else {
                        fields.push((key, value));
                    }
                }
                Ok(OrderedJsonValue::Object(fields))
            }
        }

        deserializer.deserialize_any(OrderedJsonVisitor)
    }
}

impl OrderedJsonValue {
    fn from_value(value: &Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(value) => Self::Bool(*value),
            Value::Number(value) => Self::Number(value.clone()),
            Value::String(value) => Self::String(value.clone()),
            Value::Array(values) => Self::Array(values.iter().map(Self::from_value).collect()),
            Value::Object(values) => Self::Object(
                values
                    .iter()
                    .map(|(key, value)| (key.clone(), Self::from_value(value)))
                    .collect(),
            ),
        }
    }

    fn object_field_mut(&mut self, name: &str) -> Option<&mut OrderedJsonValue> {
        let Self::Object(fields) = self else {
            return None;
        };
        fields
            .iter_mut()
            .find(|(field_name, _)| field_name == name)
            .map(|(_, value)| value)
    }

    fn set_object_field(&mut self, name: &str, value: OrderedJsonValue) -> Result<(), String> {
        let Self::Object(fields) = self else {
            return Err("ASAR ordered header object is malformed.".to_string());
        };
        if let Some((_, existing)) = fields.iter_mut().find(|(field_name, _)| field_name == name) {
            *existing = value;
        } else {
            fields.push((name.to_string(), value));
        }
        Ok(())
    }
}

fn write_ordered_json(value: &OrderedJsonValue, output: &mut Vec<u8>) -> Result<(), String> {
    match value {
        OrderedJsonValue::Null => output.extend_from_slice(b"null"),
        OrderedJsonValue::Bool(true) => output.extend_from_slice(b"true"),
        OrderedJsonValue::Bool(false) => output.extend_from_slice(b"false"),
        OrderedJsonValue::Number(value) => output.extend_from_slice(value.to_string().as_bytes()),
        OrderedJsonValue::String(value) => serde_json::to_writer(&mut *output, value)
            .map_err(|error| format!("序列化 ASAR header 字符串失败: {error}"))?,
        OrderedJsonValue::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_ordered_json(value, output)?;
            }
            output.push(b']');
        }
        OrderedJsonValue::Object(fields) => {
            output.push(b'{');
            for (index, (name, value)) in fields.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                serde_json::to_writer(&mut *output, name)
                    .map_err(|error| format!("序列化 ASAR header key 失败: {error}"))?;
                output.push(b':');
                write_ordered_json(value, output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}

fn serialize_ordered_json(value: &OrderedJsonValue) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    write_ordered_json(value, &mut output)?;
    Ok(output)
}

fn serialize_asar_header(asar: &AsarHeader) -> Result<Vec<u8>, String> {
    serialize_ordered_json(&asar.ordered_header)
}

#[derive(Debug)]
struct PatchTarget {
    path: String,
    entry_path: Vec<String>,
    entry: Value,
    offset: u64,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct WriteResult {
    header_hash: String,
    patched_files: Vec<String>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn count_occurrences(text: &str, needle: &str) -> usize {
    text.match_indices(needle).count()
}

fn replace_range(text: &str, start: usize, end: usize, replacement: &str) -> String {
    format!("{}{}{}", &text[..start], replacement, &text[end..])
}

fn pad_to_byte_length(text: String, expected_length: usize) -> Option<String> {
    let current_length = text.as_bytes().len();
    (current_length <= expected_length).then(|| {
        let mut padded = text;
        padded.push_str(&" ".repeat(expected_length - current_length));
        padded
    })
}

fn pad_same_size_replacement(before: &str, after: &str) -> Option<String> {
    let before_length = before.as_bytes().len();
    let after_length = after.as_bytes().len();
    (after_length <= before_length).then(|| {
        let mut replacement = after.to_string();
        replacement.push_str(&" ".repeat(before_length - after_length));
        replacement
    })
}

fn is_stable_metadata_retry_asset(text: &str) -> bool {
    text.contains("stable-metadata") && text.contains("Unknown method: process/spawn")
}

fn replace_stable_metadata_non_retry_guard(text: &str) -> TextPatch {
    let guard = replace_exact_same_size_label(
        text,
        STABLE_METADATA_NON_RETRY_BEFORE,
        STABLE_METADATA_NON_RETRY_AFTER,
    );
    let mut patched_text = match guard.state {
        PatchDisposition::Patchable => guard.text.expect("patchable stable metadata guard"),
        PatchDisposition::Patched => text.to_string(),
        state => return TextPatch::state(state),
    };
    let mut changed = guard.state == PatchDisposition::Patchable;
    if patched_text.contains(HISTORY_PROVIDER_FILTER_BEFORE) {
        patched_text = patched_text.replace(
            HISTORY_PROVIDER_FILTER_BEFORE,
            HISTORY_PROVIDER_FILTER_AFTER,
        );
        changed = true;
    }
    if changed {
        TextPatch::patchable(patched_text)
    } else {
        TextPatch::patched()
    }
}

fn read_asar_header(asar_path: &Path) -> Result<AsarHeader, String> {
    let metadata = fs::metadata(asar_path)
        .map_err(|error| format!("读取 app.asar 元数据失败 {}: {error}", asar_path.display()))?;
    if metadata.len() < 16 {
        return Err(format!("app.asar 头部过短: {}", asar_path.display()));
    }

    let mut file = File::open(asar_path)
        .map_err(|error| format!("打开 app.asar 失败 {}: {error}", asar_path.display()))?;
    let mut prefix = [0_u8; 16];
    file.read_exact(&mut prefix)
        .map_err(|error| format!("读取 app.asar 头部失败 {}: {error}", asar_path.display()))?;
    let header_pickle_size =
        u32::from_le_bytes(prefix[4..8].try_into().expect("slice length")) as u64;
    let header_size = u32::from_le_bytes(prefix[12..16].try_into().expect("slice length")) as usize;
    let header_end = 16_u64
        .checked_add(header_size as u64)
        .ok_or_else(|| "ASAR 头部偏移溢出。".to_string())?;
    let files_offset = 8_u64
        .checked_add(header_pickle_size)
        .ok_or_else(|| "ASAR 文件区偏移溢出。".to_string())?;
    let padding_size = files_offset
        .checked_sub(header_end)
        .filter(|padding_size| *padding_size <= 3)
        .ok_or_else(|| "ASAR header pickle 布局无效。".to_string())?;
    if files_offset > metadata.len() {
        return Err(format!(
            "app.asar JSON header 超出文件范围: {}",
            asar_path.display()
        ));
    }

    let mut header_bytes = vec![0_u8; header_size];
    file.read_exact(&mut header_bytes).map_err(|error| {
        format!(
            "读取 app.asar JSON header 失败 {}: {error}",
            asar_path.display()
        )
    })?;
    if padding_size > 0 {
        let mut padding = vec![0_u8; padding_size as usize];
        file.read_exact(&mut padding).map_err(|error| {
            format!(
                "读取 app.asar header padding 失败 {}: {error}",
                asar_path.display()
            )
        })?;
        if padding.iter().any(|byte| *byte != 0) {
            return Err(format!(
                "app.asar header padding 无效: {}",
                asar_path.display()
            ));
        }
    }
    let header = serde_json::from_slice(&header_bytes).map_err(|error| {
        format!(
            "解析 app.asar JSON header 失败 {}: {error}",
            asar_path.display()
        )
    })?;
    let ordered_header = serde_json::from_slice(&header_bytes).map_err(|error| {
        format!(
            "解析保序 app.asar JSON header 失败 {}: {error}",
            asar_path.display()
        )
    })?;

    Ok(AsarHeader {
        header_bytes,
        header,
        ordered_header,
        files_offset,
    })
}

fn list_asar_files(header: &Value) -> Result<Vec<AsarFile>, String> {
    fn walk(node: &Value, parts: &mut Vec<String>, out: &mut Vec<AsarFile>) -> Result<(), String> {
        let Some(object) = node.as_object() else {
            return Ok(());
        };
        if let Some(files) = object.get("files").and_then(Value::as_object) {
            for (name, child) in files {
                parts.push(name.clone());
                walk(child, parts, out)?;
                parts.pop();
            }
            return Ok(());
        }

        let Some(offset) = object.get("offset").and_then(Value::as_str) else {
            return Ok(());
        };
        let Some(size) = object.get("size").and_then(Value::as_u64) else {
            return Ok(());
        };
        let offset = offset
            .parse::<u64>()
            .map_err(|error| format!("ASAR 条目 offset 无效 {offset:?}: {error}"))?;
        let size =
            usize::try_from(size).map_err(|_| "ASAR 条目 size 超出平台范围。".to_string())?;
        out.push(AsarFile {
            path: parts.join("/"),
            entry_path: parts.clone(),
            entry: node.clone(),
            offset,
            size,
        });
        Ok(())
    }

    let mut files = Vec::new();
    walk(header, &mut Vec::new(), &mut files)?;
    Ok(files)
}

fn read_asar_entry(
    asar_path: &Path,
    files_offset: u64,
    entry: &AsarFile,
) -> Result<(u64, Vec<u8>), String> {
    let offset = files_offset
        .checked_add(entry.offset)
        .ok_or_else(|| format!("ASAR 条目偏移溢出: {}", entry.path))?;
    let end = offset
        .checked_add(entry.size as u64)
        .ok_or_else(|| format!("ASAR 条目长度溢出: {}", entry.path))?;
    let metadata = fs::metadata(asar_path)
        .map_err(|error| format!("读取 app.asar 元数据失败 {}: {error}", asar_path.display()))?;
    if end > metadata.len() {
        return Err(format!("ASAR 条目超出文件范围: {}", entry.path));
    }

    let mut file = File::open(asar_path)
        .map_err(|error| format!("打开 app.asar 失败 {}: {error}", asar_path.display()))?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| format!("定位 ASAR 条目失败 {}: {error}", entry.path))?;
    let mut bytes = vec![0_u8; entry.size];
    file.read_exact(&mut bytes)
        .map_err(|error| format!("读取 ASAR 条目失败 {}: {error}", entry.path))?;
    Ok((offset, bytes))
}

fn asar_entry_mut<'a>(header: &'a mut Value, components: &[String]) -> Option<&'a mut Value> {
    let mut current = header;
    for component in components {
        let files = current.get_mut("files")?.as_object_mut()?;
        current = files.get_mut(component)?;
    }
    Some(current)
}

fn ordered_asar_entry_mut<'a>(
    header: &'a mut OrderedJsonValue,
    components: &[String],
) -> Option<&'a mut OrderedJsonValue> {
    let mut current = header;
    for component in components {
        current = current.object_field_mut("files")?;
        current = current.object_field_mut(component)?;
    }
    Some(current)
}

fn sync_ordered_entry_integrity(
    ordered_entry: &mut OrderedJsonValue,
    entry: &Value,
) -> Result<(), String> {
    let Some(integrity) = entry.get("integrity") else {
        return Ok(());
    };
    let Some(integrity) = integrity.as_object() else {
        return Ok(());
    };
    let ordered_integrity = ordered_entry
        .object_field_mut("integrity")
        .ok_or_else(|| "ASAR ordered header 中缺少 integrity。".to_string())?;
    for field in ["algorithm", "hash", "blocks"] {
        let value = integrity
            .get(field)
            .ok_or_else(|| format!("更新后的 ASAR integrity 缺少 {field}。"))?;
        ordered_integrity.set_object_field(field, OrderedJsonValue::from_value(value))?;
    }
    Ok(())
}

fn json_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn js_array_to_string(values: &[Value]) -> String {
    values
        .iter()
        .map(|value| match value {
            Value::Null => String::new(),
            Value::Bool(value) => value.to_string(),
            Value::Number(value) => value.to_string(),
            Value::String(value) => value.clone(),
            Value::Array(values) => js_array_to_string(values),
            Value::Object(_) => "[object Object]".to_string(),
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn js_number_from_string(value: &str) -> Option<f64> {
    let value = value.trim();
    if value.is_empty() {
        return Some(0.0);
    }
    if !value.starts_with(['+', '-']) {
        for (prefix, radix) in [
            ("0x", 16),
            ("0X", 16),
            ("0b", 2),
            ("0B", 2),
            ("0o", 8),
            ("0O", 8),
        ] {
            if let Some(digits) = value.strip_prefix(prefix) {
                return u64::from_str_radix(digits, radix)
                    .ok()
                    .map(|value| value as f64);
            }
        }
    }
    match value {
        "Infinity" | "+Infinity" => Some(f64::INFINITY),
        "-Infinity" => Some(f64::NEG_INFINITY),
        _ => value.parse::<f64>().ok(),
    }
}

fn js_number_from_json(value: &Value) -> Option<f64> {
    match value {
        Value::Null => Some(0.0),
        Value::Bool(false) => Some(0.0),
        Value::Bool(true) => Some(1.0),
        Value::Number(number) => number.as_f64(),
        Value::String(number) => js_number_from_string(number),
        Value::Array(values) => js_number_from_string(&js_array_to_string(values)),
        Value::Object(_) => None,
    }
}

fn valid_block_size(value: Option<&Value>) -> Result<usize, String> {
    let block_size = match value {
        Some(value) => js_number_from_json(value),
        None => Some(INTEGRITY_DEFAULT_BLOCK_SIZE as f64),
    }
    .ok_or_else(|| "ASAR integrity blockSize 无效。".to_string())?;
    if !block_size.is_finite()
        || block_size <= 0.0
        || block_size.fract() != 0.0
        || block_size > 9_007_199_254_740_991.0
    {
        return Err("ASAR integrity blockSize 超出安全整数范围。".to_string());
    }
    let block_size = block_size as u64;
    usize::try_from(block_size).map_err(|_| "ASAR integrity blockSize 超出平台范围。".to_string())
}

fn integrity_block_size_for_update(integrity: &Map<String, Value>) -> Result<usize, String> {
    let value = integrity
        .get("blockSize")
        .filter(|value| json_truthy(value));
    valid_block_size(value)
}

fn integrity_block_size_for_verify(integrity: &Map<String, Value>) -> Result<usize, String> {
    // JS uses `blockSize ?? default` for verification, unlike the update path's
    // `blockSize || default`, so only null and missing values use the default.
    let value = integrity.get("blockSize").filter(|value| !value.is_null());
    valid_block_size(value)
}

fn integrity_blocks(bytes: &[u8], block_size: usize) -> Vec<String> {
    if bytes.is_empty() {
        return vec![sha256_hex(&[])];
    }
    bytes.chunks(block_size).map(sha256_hex).collect()
}

fn update_entry_integrity(entry: &mut Value, bytes: &[u8]) -> Result<(), String> {
    let Some(integrity) = entry.get_mut("integrity") else {
        return Ok(());
    };
    if integrity.is_array() {
        return Err("ASAR integrity 不能是数组。".to_string());
    }
    let Some(integrity) = integrity.as_object_mut() else {
        return Ok(());
    };
    let block_size = integrity_block_size_for_update(integrity)?;
    let blocks = integrity_blocks(bytes, block_size)
        .into_iter()
        .map(Value::String)
        .collect::<Vec<_>>();

    let algorithm_missing_or_false = integrity
        .get("algorithm")
        .map_or(true, |algorithm| !json_truthy(algorithm));
    if algorithm_missing_or_false {
        integrity.insert("algorithm".to_string(), Value::String("SHA256".to_string()));
    }
    integrity.insert("hash".to_string(), Value::String(sha256_hex(bytes)));
    integrity.insert("blocks".to_string(), Value::Array(blocks));
    Ok(())
}

fn verify_entry_integrity(entry: &Value, bytes: &[u8]) -> bool {
    let Some(integrity) = entry.get("integrity") else {
        return true;
    };
    if integrity.is_array() {
        return false;
    }
    let Some(integrity) = integrity.as_object() else {
        return true;
    };
    let Ok(block_size) = integrity_block_size_for_verify(integrity) else {
        return false;
    };
    if let Some(algorithm) = integrity
        .get("algorithm")
        .filter(|value| json_truthy(value))
    {
        let algorithm = match algorithm {
            Value::String(value) => value.to_uppercase(),
            Value::Number(value) => value.to_string().to_uppercase(),
            Value::Bool(value) => value.to_string().to_uppercase(),
            _ => return false,
        };
        if algorithm != "SHA256" {
            return false;
        }
    }

    let blocks = integrity_blocks(bytes, block_size);
    integrity.get("hash").and_then(Value::as_str) == Some(sha256_hex(bytes).as_str())
        && integrity
            .get("blocks")
            .and_then(Value::as_array)
            .is_some_and(|actual_blocks| {
                actual_blocks.len() == blocks.len()
                    && actual_blocks
                        .iter()
                        .zip(blocks.iter())
                        .all(|(actual, expected)| actual.as_str() == Some(expected))
            })
}

fn write_patched_asar(
    asar_path: &Path,
    asar: &mut AsarHeader,
    targets: &mut [PatchTarget],
) -> Result<WriteResult, String> {
    for target in targets.iter_mut() {
        update_entry_integrity(&mut target.entry, &target.bytes)?;
        let header_entry = asar_entry_mut(&mut asar.header, &target.entry_path)
            .ok_or_else(|| format!("ASAR header 中缺少目标条目: {}", target.path))?;
        *header_entry = target.entry.clone();
        let ordered_header_entry =
            ordered_asar_entry_mut(&mut asar.ordered_header, &target.entry_path)
                .ok_or_else(|| format!("ASAR ordered header 中缺少目标条目: {}", target.path))?;
        sync_ordered_entry_integrity(ordered_header_entry, &target.entry)?;
    }

    let new_header_bytes = serialize_asar_header(asar)?;
    if new_header_bytes.len() != asar.header_bytes.len() {
        return Err("ASAR header 长度变化，拒绝原地写入。".to_string());
    }

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(asar_path)
        .map_err(|error| {
            format!(
                "以写模式打开 app.asar 失败 {}: {error}",
                asar_path.display()
            )
        })?;
    for target in targets.iter() {
        file.seek(SeekFrom::Start(target.offset))
            .map_err(|error| format!("定位待写 ASAR 条目失败 {}: {error}", target.path))?;
        file.write_all(&target.bytes)
            .map_err(|error| format!("写入 ASAR 条目失败 {}: {error}", target.path))?;
    }
    file.seek(SeekFrom::Start(16))
        .map_err(|error| format!("定位 ASAR header 失败: {error}"))?;
    file.write_all(&new_header_bytes)
        .map_err(|error| format!("写入 ASAR header 失败: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("刷新 app.asar 失败 {}: {error}", asar_path.display()))?;

    Ok(WriteResult {
        header_hash: sha256_hex(&new_header_bytes),
        patched_files: targets.iter().map(|target| target.path.clone()).collect(),
    })
}

fn decode_embedded_payload(
    base64_payload: &str,
    expected_hash: &str,
    label: &str,
) -> Result<String, String> {
    let compressed = base64::engine::general_purpose::STANDARD
        .decode(base64_payload)
        .map_err(|error| format!("解码内嵌 {label} payload 失败: {error}"))?;
    let mut decoder = GzDecoder::new(compressed.as_slice());
    let mut bytes = Vec::new();
    decoder
        .read_to_end(&mut bytes)
        .map_err(|error| format!("解压内嵌 {label} payload 失败: {error}"))?;
    if sha256_hex(&bytes) != expected_hash {
        return Err(format!("内嵌 {label} payload 的 SHA256 校验失败。"));
    }
    String::from_utf8(bytes).map_err(|error| format!("内嵌 {label} payload 不是 UTF-8: {error}"))
}

fn embedded_custom_picker_component() -> Result<String, String> {
    decode_embedded_payload(
        CUSTOM_PICKER_COMPONENT_GZIP_BASE64,
        CUSTOM_PICKER_COMPONENT_SHA256,
        "model picker component",
    )
}

fn embedded_custom_picker_css() -> Result<String, String> {
    decode_embedded_payload(
        CUSTOM_PICKER_CSS_GZIP_BASE64,
        CUSTOM_PICKER_CSS_SHA256,
        "model picker CSS",
    )
}

fn replace_model_picker_gate(text: &str) -> TextPatch {
    let pending_patterns = [
        format!(r"([,;]{IDENT}=)([A-Za-z_$])(&&{IDENT}!==`amazonBedrock`[,;])"),
        format!(r#"([,;]{IDENT}=)([A-Za-z_$])(&&{IDENT}!=="amazonBedrock"[,;])"#),
    ];
    let applied_patterns = [
        format!(r"[,;]{IDENT}=0&&{IDENT}!==`amazonBedrock`[,;]"),
        format!(r#"[,;]{IDENT}=0&&{IDENT}!=="amazonBedrock"[,;]"#),
    ];

    let applied_count = applied_patterns
        .iter()
        .map(|pattern| Regex::new(pattern).expect("valid applied model-gate regex"))
        .map(|regex| regex.find_iter(text).count())
        .sum::<usize>();
    let mut pending = Vec::new();
    for pattern in &pending_patterns {
        let regex = Regex::new(pattern).expect("valid pending model-gate regex");
        for captures in regex.captures_iter(text) {
            let whole = captures.get(0).expect("whole match");
            pending.push((
                whole.start(),
                whole.end(),
                captures
                    .get(1)
                    .expect("prefix capture")
                    .as_str()
                    .to_string(),
                captures
                    .get(3)
                    .expect("suffix capture")
                    .as_str()
                    .to_string(),
            ));
        }
    }
    if applied_count > 1 || pending.len() > 1 || (applied_count == 1 && pending.len() == 1) {
        return TextPatch::state(PatchDisposition::Ambiguous);
    }
    if applied_count == 1 {
        return TextPatch::patched();
    }
    if let Some((start, end, prefix, suffix)) = pending.into_iter().next() {
        return TextPatch::patchable(replace_range(
            text,
            start,
            end,
            &format!("{prefix}0{suffix}"),
        ));
    }

    let exact_patches = [
        (",s=i&&e!==`amazonBedrock`;", ",s=0&&e!==`amazonBedrock`;"),
        (",u=s&&e!==`amazonBedrock`,", ",u=0&&e!==`amazonBedrock`,"),
        (
            ",s=i&&e!==\"amazonBedrock\";",
            ",s=0&&e!==\"amazonBedrock\";",
        ),
        (
            ",u=s&&e!==\"amazonBedrock\",",
            ",u=0&&e!==\"amazonBedrock\",",
        ),
    ];
    let applied_exact_count = exact_patches
        .iter()
        .filter(|(_, after)| text.contains(after))
        .count();
    let mut pending_exact_count = 0;
    let mut single_pending_exact = None;
    for (before, after) in exact_patches {
        let count = count_occurrences(text, before);
        pending_exact_count += count;
        if count == 1 {
            single_pending_exact = Some((before, after));
        }
    }
    if applied_exact_count > 1
        || pending_exact_count > 1
        || (applied_exact_count == 1 && pending_exact_count == 1)
    {
        return TextPatch::state(PatchDisposition::Ambiguous);
    }
    if applied_exact_count == 1 {
        return TextPatch::patched();
    }
    if let Some((before, after)) = single_pending_exact {
        return TextPatch::patchable(text.replacen(before, after, 1));
    }
    TextPatch::state(PatchDisposition::Missing)
}

#[derive(Debug)]
struct MaxReasoningMatch {
    start: usize,
    end: usize,
    effort: String,
    enabled: String,
    supported: String,
}

fn max_reasoning_matches(text: &str, after: bool) -> Vec<MaxReasoningMatch> {
    let pattern = if after {
        format!(
            r"\(\{{reasoningEffort:({IDENT})\}}\)=>({IDENT})\(({IDENT})\)&&\(({IDENT})===`max`\|\|({IDENT})\.has\(({IDENT})\)\)"
        )
    } else {
        format!(
            r"\(\{{reasoningEffort:({IDENT})\}}\)=>({IDENT})\(({IDENT})\)&&({IDENT})\.has\(({IDENT})\)"
        )
    };
    let regex = Regex::new(&pattern).expect("valid max-reasoning regex");
    regex
        .captures_iter(text)
        .filter_map(|captures| {
            let whole = captures.get(0)?;
            let effort = captures.get(1)?.as_str();
            let enabled = captures.get(2)?.as_str();
            let called_effort = captures.get(3)?.as_str();
            if after {
                let comparison_effort = captures.get(4)?.as_str();
                let supported = captures.get(5)?.as_str();
                let has_effort = captures.get(6)?.as_str();
                (effort == called_effort && effort == comparison_effort && effort == has_effort)
                    .then(|| MaxReasoningMatch {
                        start: whole.start(),
                        end: whole.end(),
                        effort: effort.to_string(),
                        enabled: enabled.to_string(),
                        supported: supported.to_string(),
                    })
            } else {
                let supported = captures.get(4)?.as_str();
                let has_effort = captures.get(5)?.as_str();
                (effort == called_effort && effort == has_effort).then(|| MaxReasoningMatch {
                    start: whole.start(),
                    end: whole.end(),
                    effort: effort.to_string(),
                    enabled: enabled.to_string(),
                    supported: supported.to_string(),
                })
            }
        })
        .collect()
}

fn replace_max_reasoning_effort_filter(text: &str) -> TextPatch {
    let applied_matches = max_reasoning_matches(text, true);
    let pending_matches = max_reasoning_matches(text, false);
    if applied_matches.len() > 1
        || pending_matches.len() > 1
        || (applied_matches.len() == 1 && pending_matches.len() == 1)
    {
        return TextPatch::state(PatchDisposition::Ambiguous);
    }
    if applied_matches.len() == 1 {
        return TextPatch::patched();
    }
    let Some(pending) = pending_matches.into_iter().next() else {
        return TextPatch::state(PatchDisposition::Missing);
    };
    let replacement = format!(
        "({{reasoningEffort:{}}})=>{}({})&&({}===`max`||{}.has({}))",
        pending.effort,
        pending.enabled,
        pending.effort,
        pending.effort,
        pending.supported,
        pending.effort,
    );
    TextPatch::patchable(replace_range(
        text,
        pending.start,
        pending.end,
        &replacement,
    ))
}

fn is_compacted_model_picker_gate(text: &str) -> bool {
    Regex::new(&format!(r",{IDENT}=0,{IDENT}={IDENT}\.some\("))
        .expect("valid compacted model-gate regex")
        .is_match(text)
}

fn compact_model_picker_gate(text: &str) -> TextPatch {
    let regex = Regex::new(&format!(
        r#"([,;])({IDENT})=0&&{IDENT}!==(?:`amazonBedrock`|"amazonBedrock")([,;])"#
    ))
    .expect("valid compact model-gate regex");
    let matches = regex.captures_iter(text).collect::<Vec<_>>();
    if matches.is_empty() {
        return TextPatch::state(PatchDisposition::Missing);
    }
    if matches.len() > 1 {
        return TextPatch::state(PatchDisposition::Ambiguous);
    }
    let captures = &matches[0];
    let whole = captures.get(0).expect("whole match");
    let replacement = format!(
        "{}{}=0{}",
        captures.get(1).expect("prefix").as_str(),
        captures.get(2).expect("variable").as_str(),
        captures.get(3).expect("suffix").as_str(),
    );
    TextPatch::patchable(replace_range(
        text,
        whole.start(),
        whole.end(),
        &replacement,
    ))
}

fn replace_model_list_filter(text: &str) -> TextPatch {
    let original_length = text.as_bytes().len();
    let gate = replace_model_picker_gate(text);
    let max_filter = replace_max_reasoning_effort_filter(gate.text.as_deref().unwrap_or(text));

    if gate.state == PatchDisposition::Missing && is_compacted_model_picker_gate(text) {
        return if max_filter.state == PatchDisposition::Patched {
            TextPatch::patched()
        } else {
            TextPatch::state(PatchDisposition::UnsafeSizeChange)
        };
    }
    if matches!(
        gate.state,
        PatchDisposition::Ambiguous
            | PatchDisposition::Missing
            | PatchDisposition::UnsafeSizeChange
    ) {
        return TextPatch::state(gate.state);
    }
    if matches!(
        max_filter.state,
        PatchDisposition::Ambiguous
            | PatchDisposition::Missing
            | PatchDisposition::UnsafeSizeChange
    ) {
        return TextPatch::state(max_filter.state);
    }
    if gate.state == PatchDisposition::Patched && max_filter.state == PatchDisposition::Patched {
        return TextPatch::patched();
    }
    if max_filter.state == PatchDisposition::Patched {
        return TextPatch::patchable(gate.text.expect("patchable gate must retain text"));
    }

    let compact_gate = compact_model_picker_gate(
        max_filter
            .text
            .as_deref()
            .expect("patchable max filter must retain text"),
    );
    if compact_gate.state != PatchDisposition::Patchable {
        return compact_gate;
    }
    let Some(padded) = pad_to_byte_length(
        compact_gate
            .text
            .expect("patchable compact gate must retain text"),
        original_length,
    ) else {
        return TextPatch::state(PatchDisposition::UnsafeSizeChange);
    };
    TextPatch::patchable(padded)
}

fn find_function_end(text: &str, open_brace_offset: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0_i32;
    let mut quote = None;
    let mut escaped = false;
    let mut line_comment = false;
    let mut block_comment = false;
    let mut index = open_brace_offset;
    while index < bytes.len() {
        let character = bytes[index];
        let next = bytes.get(index + 1).copied();
        if line_comment {
            if character == b'\n' || character == b'\r' {
                line_comment = false;
            }
            index += 1;
            continue;
        }
        if block_comment {
            if character == b'*' && next == Some(b'/') {
                block_comment = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == b'\\' {
                escaped = true;
            } else if character == active_quote {
                quote = None;
            }
            index += 1;
            continue;
        }
        if character == b'/' && next == Some(b'/') {
            line_comment = true;
            index += 2;
            continue;
        }
        if character == b'/' && next == Some(b'*') {
            block_comment = true;
            index += 2;
            continue;
        }
        if matches!(character, b'\'' | b'\"' | b'`') {
            quote = Some(character);
            index += 1;
            continue;
        }
        if character == b'{' {
            depth += 1;
        } else if character == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(index + 1);
            }
        }
        index += 1;
    }
    None
}

#[derive(Debug)]
struct PowerSliderFunction {
    name: String,
    react_alias: String,
    source: String,
    start: usize,
    end: usize,
}

fn find_power_slider_function(text: &str) -> Result<PowerSliderFunction, PatchDisposition> {
    let export_regex = Regex::new(&format!(
        r"export\{{({IDENT}) as ModelPickerPowerSliderImpl\}}"
    ))
    .expect("valid power-slider export regex");
    let exports = export_regex.captures_iter(text).collect::<Vec<_>>();
    if exports.is_empty() {
        return Err(PatchDisposition::Missing);
    }
    if exports.len() > 1 {
        return Err(PatchDisposition::Ambiguous);
    }
    let name = exports[0].get(1).expect("export name").as_str().to_string();
    let signature = format!("function {name}(");
    let mut starts = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = text[cursor..].find(&signature) {
        let start = cursor + relative;
        starts.push(start);
        cursor = start + signature.len();
    }
    if starts.is_empty() {
        return Err(PatchDisposition::Missing);
    }
    if starts.len() > 1 {
        return Err(PatchDisposition::Ambiguous);
    }
    let start = starts[0];
    let Some(relative_open_brace) = text[start + signature.len()..].find('{') else {
        return Err(PatchDisposition::Missing);
    };
    let open_brace = start + signature.len() + relative_open_brace;
    let Some(end) = find_function_end(text, open_brace) else {
        return Err(PatchDisposition::Missing);
    };
    let source = text[start..end].to_string();
    let react_alias_regex =
        Regex::new(&format!(r"\b({IDENT})\.useState\b")).expect("valid React alias regex");
    let react_aliases = react_alias_regex
        .captures_iter(&source)
        .map(|captures| captures.get(1).expect("React alias").as_str().to_string())
        .collect::<BTreeSet<_>>();
    if react_aliases.len() != 1 {
        return Err(if react_aliases.len() > 1 {
            PatchDisposition::Ambiguous
        } else {
            PatchDisposition::Missing
        });
    }
    Ok(PowerSliderFunction {
        name,
        react_alias: react_aliases
            .into_iter()
            .next()
            .expect("single React alias"),
        source,
        start,
        end,
    })
}

fn trim_one_trailing_line_ending(value: String) -> String {
    if let Some(value) = value.strip_suffix("\r\n") {
        value.to_string()
    } else if let Some(value) = value.strip_suffix('\n') {
        value.to_string()
    } else {
        value
    }
}

fn replace_power_slider_component(text: &str) -> Result<TextPatch, String> {
    let marker_count = count_occurrences(text, CUSTOM_PICKER_UI_MARKER);
    if marker_count > 1 {
        return Ok(TextPatch::state(PatchDisposition::Ambiguous));
    }
    let target = match find_power_slider_function(text) {
        Ok(target) => target,
        Err(state) => return Ok(TextPatch::state(state)),
    };
    if marker_count == 1 {
        let react_binding_regex = Regex::new(&format!(
            r#""{}";const {}=({IDENT}),"#,
            regex::escape(CUSTOM_PICKER_UI_MARKER),
            regex::escape(&target.react_alias),
        ))
        .expect("valid custom component binding regex");
        let Some(react_binding) = react_binding_regex.captures(&target.source) else {
            return Ok(TextPatch::state(PatchDisposition::UnsupportedPreimage));
        };
        let component = trim_one_trailing_line_ending(
            embedded_custom_picker_component()?
                .replacen("__COMPONENT__", &target.name, 1)
                .replace(
                    "__REACT__",
                    react_binding.get(1).expect("React binding").as_str(),
                ),
        );
        return Ok(if target.source == component {
            TextPatch::patched()
        } else {
            TextPatch::state(PatchDisposition::UnsupportedPreimage)
        });
    }
    if sha256_hex(target.source.as_bytes()) != ORIGINAL_POWER_SLIDER_FUNCTION_SHA256 {
        return Ok(TextPatch::state(PatchDisposition::UnsupportedPreimage));
    }
    let component = embedded_custom_picker_component()?
        .replacen("__COMPONENT__", &target.name, 1)
        .replace("__REACT__", &target.react_alias);
    let Some(replacement) = pad_same_size_replacement(&target.source, &component) else {
        return Ok(TextPatch::state(PatchDisposition::UnsafeSizeChange));
    };
    Ok(TextPatch::patchable(replace_range(
        text,
        target.start,
        target.end,
        &replacement,
    )))
}

fn replace_power_slider_css(text: &str) -> Result<TextPatch, String> {
    let marker_count = count_occurrences(text, CUSTOM_PICKER_CSS_MARKER);
    if marker_count > 1 {
        return Ok(TextPatch::state(PatchDisposition::Ambiguous));
    }
    let css = embedded_custom_picker_css()?;
    if marker_count == 1 {
        return Ok(
            if text.starts_with(&css) && text[css.len()..].trim().is_empty() {
                TextPatch::patched()
            } else {
                TextPatch::state(PatchDisposition::UnsupportedPreimage)
            },
        );
    }
    if sha256_hex(text.as_bytes()) != ORIGINAL_POWER_SLIDER_CSS_SHA256 {
        return Ok(TextPatch::state(PatchDisposition::UnsupportedPreimage));
    }
    let Some(replacement) = pad_same_size_replacement(text, &css) else {
        return Ok(TextPatch::state(PatchDisposition::UnsafeSizeChange));
    };
    Ok(TextPatch::patchable(replacement))
}

fn is_power_slider_js_entry(entry_path: &str) -> bool {
    entry_path
        .strip_prefix("webview/assets/model-picker-power-slider-impl-")
        .is_some_and(|suffix| suffix.ends_with(".js"))
}

fn is_power_slider_css_entry(entry_path: &str) -> bool {
    entry_path
        .strip_prefix("webview/assets/model-picker-power-slider-impl-")
        .is_some_and(|suffix| suffix.ends_with(".css"))
}

fn power_selections_replacement(assignment: &str) -> String {
    format!(
        "{assignment}=[{POWER_SELECTIONS_PATCH_MARKER}...{MANAGED_POWER_SELECTIONS_EXPRESSION}]"
    )
}

fn replace_collapsed_power_selection_label(text: &str) -> TextPatch {
    let before_regex = Regex::new(
        r#"function ([A-Za-z_$][A-Za-z0-9_$]*)\(([A-Za-z_$][A-Za-z0-9_$]*),([A-Za-z_$][A-Za-z0-9_$]*)\)\{return`\$\{([A-Za-z_$][A-Za-z0-9_$]*)\.modelLabel\} \$\{([A-Za-z_$][A-Za-z0-9_$]*)\.formatMessage\(([A-Za-z_$][A-Za-z0-9_$]*)\[([A-Za-z_$][A-Za-z0-9_$]*)\.reasoningEffort\]\)\}`\}"#,
    )
    .expect("valid original collapsed Power label regex");
    let after_regex = Regex::new(
        r#"function ([A-Za-z_$][A-Za-z0-9_$]*)\(([A-Za-z_$][A-Za-z0-9_$]*),([A-Za-z_$][A-Za-z0-9_$]*)\)\{return`\$\{([A-Za-z_$][A-Za-z0-9_$]*)\.modelLabel\} \$\{([A-Za-z_$][A-Za-z0-9_$]*)\[([A-Za-z_$][A-Za-z0-9_$]*)\.reasoningEffort\]\}`\}"#,
    )
    .expect("valid patched collapsed Power label regex");
    let before_matches = before_regex
        .captures_iter(text)
        .filter(|captures| {
            captures.get(2).map(|value| value.as_str())
                == captures.get(4).map(|value| value.as_str())
                && captures.get(2).map(|value| value.as_str())
                    == captures.get(7).map(|value| value.as_str())
                && captures.get(3).map(|value| value.as_str())
                    == captures.get(5).map(|value| value.as_str())
        })
        .collect::<Vec<_>>();
    let after_matches = after_regex
        .captures_iter(text)
        .filter(|captures| {
            captures.get(2).map(|value| value.as_str())
                == captures.get(4).map(|value| value.as_str())
                && captures.get(2).map(|value| value.as_str())
                    == captures.get(6).map(|value| value.as_str())
        })
        .collect::<Vec<_>>();

    if before_matches.len() > 1
        || after_matches.len() > 1
        || (!before_matches.is_empty() && !after_matches.is_empty())
    {
        return TextPatch::state(PatchDisposition::Ambiguous);
    }
    if let Some(applied) = after_matches.first() {
        let labels_variable = applied.get(5).expect("labels variable").as_str();
        let assignment = format!("{labels_variable}={COLLAPSED_REASONING_LABELS_JSON}");
        return match count_occurrences(text, &assignment) {
            0 => TextPatch::state(PatchDisposition::Missing),
            1 => TextPatch::patched(),
            _ => TextPatch::state(PatchDisposition::Ambiguous),
        };
    }
    let Some(original) = before_matches.first() else {
        return TextPatch::state(PatchDisposition::Missing);
    };

    let labels_variable = original.get(6).expect("labels variable").as_str();
    let assignment_prefix = format!("{labels_variable}=");
    let formatter_end = original.get(0).expect("whole formatter").end();
    let assignment_candidates = text[formatter_end..]
        .match_indices(&assignment_prefix)
        .filter_map(|(relative_offset, _)| {
            let assignment_offset = formatter_end + relative_offset;
            let object_offset = assignment_offset + assignment_prefix.len();
            if text.as_bytes().get(object_offset) != Some(&b'{') {
                return None;
            }
            let object_end = find_function_end(text, object_offset)?;
            let source = &text[object_offset..object_end];
            (source.contains("composer.modelPicker.work.reasoning.standard.label")
                && source.contains("composer.modelPicker.work.reasoning.extended.label"))
            .then_some((object_offset, object_end))
        })
        .collect::<Vec<_>>();
    if assignment_candidates.len() != 1 {
        return TextPatch::state(if assignment_candidates.len() > 1 {
            PatchDisposition::Ambiguous
        } else {
            PatchDisposition::Missing
        });
    }
    let (object_offset, object_end) = assignment_candidates[0];
    let original_labels = &text[object_offset..object_end];

    let function_name = original.get(1).expect("formatter name").as_str();
    let model_argument = original.get(2).expect("model argument").as_str();
    let intl_argument = original.get(3).expect("intl argument").as_str();
    let formatter = format!(
        "function {function_name}({model_argument},{intl_argument}){{return`${{{model_argument}.modelLabel}} ${{{labels_variable}[{model_argument}.reasoningEffort]}}`}}"
    );
    let whole_formatter = original.get(0).expect("whole formatter");
    let Some(padded_formatter) = pad_same_size_replacement(whole_formatter.as_str(), &formatter)
    else {
        return TextPatch::state(PatchDisposition::UnsafeSizeChange);
    };
    let Some(padded_labels) =
        pad_same_size_replacement(original_labels, COLLAPSED_REASONING_LABELS_JSON)
    else {
        return TextPatch::state(PatchDisposition::UnsafeSizeChange);
    };

    let patched = replace_range(
        text,
        whole_formatter.start(),
        whole_formatter.end(),
        &padded_formatter,
    );
    TextPatch::patchable(replace_range(
        &patched,
        object_offset,
        object_end,
        &padded_labels,
    ))
}

fn find_matching_array_bracket(text: &str, open_offset: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0_i32;
    let mut quote = None;
    let mut escaped = false;
    let mut index = open_offset;
    while index < bytes.len() {
        let character = bytes[index];
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == b'\\' {
                escaped = true;
            } else if character == active_quote {
                quote = None;
            }
            index += 1;
            continue;
        }
        if matches!(character, b'\'' | b'\"' | b'`') {
            quote = Some(character);
            index += 1;
            continue;
        }
        if character == b'[' {
            depth += 1;
        } else if character == b']' {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
        index += 1;
    }
    None
}

#[derive(Debug)]
struct PowerSelectionsRegion {
    assignment: String,
    start: usize,
    end: usize,
}

fn current_power_selections_region(text: &str) -> Result<PowerSelectionsRegion, PatchDisposition> {
    let marker_count = count_occurrences(text, POWER_SELECTIONS_PATCH_MARKER);
    if marker_count == 0 {
        return Err(PatchDisposition::Missing);
    }
    if marker_count > 1 {
        return Err(PatchDisposition::Ambiguous);
    }
    let marker_offset = text
        .find(POWER_SELECTIONS_PATCH_MARKER)
        .expect("counted marker must have an offset");
    let prefix = &text[..marker_offset];
    let assignment_regex = Regex::new(&format!(r"({IDENT})=\[[ \t]*$"))
        .expect("valid power selection assignment regex");
    let Some(assignment) = assignment_regex.captures(prefix) else {
        return Err(PatchDisposition::Missing);
    };
    let full_assignment = assignment.get(0).expect("full assignment");
    let start = full_assignment.start();
    let array_open = start
        + full_assignment
            .as_str()
            .find('[')
            .expect("array assignment");
    let Some(array_close) = find_matching_array_bracket(text, array_open) else {
        return Err(PatchDisposition::Missing);
    };
    Ok(PowerSelectionsRegion {
        assignment: assignment
            .get(1)
            .expect("assignment variable")
            .as_str()
            .to_string(),
        start,
        end: array_close + 1,
    })
}

fn validate_current_power_selections(text: &str) -> TextPatch {
    let region = match current_power_selections_region(text) {
        Ok(region) => region,
        Err(state) => return TextPatch::state(state),
    };
    let expected = power_selections_replacement(&region.assignment);
    if text[region.start..region.end] == expected {
        TextPatch::patched()
    } else {
        TextPatch::state(PatchDisposition::UnsupportedPreimage)
    }
}

fn replace_marked_power_selections(text: &str, marker: &str) -> TextPatch {
    let count = count_occurrences(text, marker);
    if count == 0 {
        return TextPatch::state(PatchDisposition::Missing);
    }
    if count > 1 {
        return TextPatch::state(PatchDisposition::Ambiguous);
    }
    let marker_offset = text
        .find(marker)
        .expect("counted marker must have an offset");
    let prefix = &text[..marker_offset];
    let assignment_regex = Regex::new(&format!(r"({IDENT})=\[[ \t]*$"))
        .expect("valid legacy power selection assignment regex");
    let Some(assignment) = assignment_regex.captures(prefix) else {
        return TextPatch::state(PatchDisposition::Missing);
    };
    let full_assignment = assignment.get(0).expect("full assignment");
    let before_offset = full_assignment.start();
    let array_open = before_offset
        + full_assignment
            .as_str()
            .find('[')
            .expect("array assignment");
    let Some(array_close) = find_matching_array_bracket(text, array_open) else {
        return TextPatch::state(PatchDisposition::Missing);
    };
    let before_end = array_close + 1;
    let before = &text[before_offset..before_end];
    let after =
        power_selections_replacement(assignment.get(1).expect("assignment variable").as_str());
    let Some(replacement) = pad_same_size_replacement(before, &after) else {
        return TextPatch::state(PatchDisposition::UnsafeSizeChange);
    };
    TextPatch::patchable(replace_range(text, before_offset, before_end, &replacement))
}

fn replace_power_selection_filter(text: &str) -> TextPatch {
    let current_filter_count = count_occurrences(text, CURRENT_LEGACY_POWER_SELECTION_FILTER)
        + count_occurrences(text, CURRENT_FALLBACK_POWER_SELECTION_FILTER)
        + count_occurrences(text, CURRENT_SPLIT_POWER_SELECTION_FILTER);
    if current_filter_count > 1 {
        return TextPatch::state(PatchDisposition::Ambiguous);
    }
    if current_filter_count == 1 {
        return TextPatch::patched();
    }

    let previous_current_regex = Regex::new(&format!(
        "{} *",
        regex::escape(PREVIOUS_CURRENT_POWER_SELECTION_FILTER)
    ))
    .expect("valid previous power-selection filter regex");
    let previous_current_matches = previous_current_regex.find_iter(text).collect::<Vec<_>>();
    if previous_current_matches.len() > 1 {
        return TextPatch::state(PatchDisposition::Ambiguous);
    }
    if let Some(previous) = previous_current_matches.first() {
        let before = previous.as_str();
        let Some(replacement) =
            pad_same_size_replacement(before, CURRENT_LEGACY_POWER_SELECTION_FILTER)
        else {
            return TextPatch::state(PatchDisposition::UnsafeSizeChange);
        };
        return TextPatch::patchable(replace_range(
            text,
            previous.start(),
            previous.end(),
            &replacement,
        ));
    }

    let mut previous_v5_matches = Vec::new();
    for (before, after) in [
        (
            PREVIOUS_V5_LEGACY_POWER_SELECTION_FILTER,
            CURRENT_LEGACY_POWER_SELECTION_FILTER,
        ),
        (
            PREVIOUS_V5_FALLBACK_POWER_SELECTION_FILTER,
            CURRENT_FALLBACK_POWER_SELECTION_FILTER,
        ),
    ] {
        let regex = Regex::new(&format!("{} *", regex::escape(before)))
            .expect("valid previous v5 Power filter regex");
        previous_v5_matches.extend(
            regex
                .find_iter(text)
                .map(|matched| (matched.start(), matched.end(), after)),
        );
    }
    if previous_v5_matches.len() > 1 {
        return TextPatch::state(PatchDisposition::Ambiguous);
    }
    if let Some((start, end, after)) = previous_v5_matches.first().copied() {
        let Some(replacement) = pad_same_size_replacement(&text[start..end], after) else {
            return TextPatch::state(PatchDisposition::UnsafeSizeChange);
        };
        return TextPatch::patchable(replace_range(text, start, end, &replacement));
    }

    let replacements = [
        (
            LEGACY_POWER_SELECTION_FILTER,
            CURRENT_LEGACY_POWER_SELECTION_FILTER,
        ),
        (
            LEGACY_FALLBACK_POWER_SELECTION_FILTER,
            CURRENT_FALLBACK_POWER_SELECTION_FILTER,
        ),
        (
            LEGACY_SPLIT_POWER_SELECTION_FILTER,
            CURRENT_SPLIT_POWER_SELECTION_FILTER,
        ),
    ];
    let matching = replacements
        .iter()
        .flat_map(|replacement| {
            std::iter::repeat(*replacement).take(count_occurrences(text, replacement.0))
        })
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return TextPatch::state(PatchDisposition::Missing);
    }
    if matching.len() > 1 {
        return TextPatch::state(PatchDisposition::Ambiguous);
    }
    let (before, after) = matching[0];
    let count = count_occurrences(text, before);
    if count == 0 {
        return TextPatch::state(PatchDisposition::Missing);
    }
    if count > 1 {
        return TextPatch::state(PatchDisposition::Ambiguous);
    }
    let Some(replacement) = pad_same_size_replacement(before, after) else {
        return TextPatch::state(PatchDisposition::UnsafeSizeChange);
    };
    TextPatch::patchable(text.replacen(before, &replacement, 1))
}

fn replace_power_picker_menu_width(text: &str) -> TextPatch {
    let marker_count = count_occurrences(text, "`cdpw`");
    if marker_count > 1 {
        return TextPatch::state(PatchDisposition::Ambiguous);
    }
    if marker_count == 1 {
        return TextPatch::patched();
    }
    let regex = Regex::new(&format!(r"({IDENT})\(`w-56`,({IDENT})\)"))
        .expect("valid Power picker width regex");
    let matches = regex.captures_iter(text).collect::<Vec<_>>();
    if matches.is_empty() {
        return TextPatch::state(PatchDisposition::Missing);
    }
    if matches.len() > 1 {
        return TextPatch::state(PatchDisposition::Ambiguous);
    }
    let captures = &matches[0];
    let whole = captures.get(0).expect("whole width match");
    let replacement = format!(
        "{}(`cdpw`,{})",
        captures.get(1).expect("width helper").as_str(),
        captures.get(2).expect("width argument").as_str(),
    );
    TextPatch::patchable(replace_range(
        text,
        whole.start(),
        whole.end(),
        &replacement,
    ))
}

fn contains_power_picker_signature(text: &str) -> bool {
    let has_split_ultra_picker = text.contains(LEGACY_SPLIT_POWER_SELECTIONS)
        && text.contains(LEGACY_SPLIT_POWER_SELECTION_FILTER);
    text.contains("gpt-5.6-terra:low")
        && text.contains("gpt-5.6-sol:xhigh")
        && text.contains("gpt-5.6-sol:ultra")
        && (text.contains("gpt-5.6-luna")
            || text.contains(LEGACY_SOL_POWER_SELECTIONS)
            || has_split_ultra_picker)
}

#[derive(Debug)]
struct ReasoningMenuVisibleLabelCandidate {
    descriptor: String,
    labels: String,
    rendered: String,
    selected_before: Vec<(usize, usize, String)>,
    selected_after_count: usize,
    menu_before: Vec<(usize, usize, String, String)>,
    menu_after_count: usize,
}

fn replace_reasoning_menu_visible_labels(text: &str) -> TextPatch {
    let anchor_regex = Regex::new(&format!(r"({IDENT})=({IDENT})\[({IDENT})\],({IDENT});"))
        .expect("valid reasoning menu label anchor regex");
    let mut candidates = Vec::new();
    for anchor in anchor_regex.captures_iter(text) {
        let descriptor = anchor.get(1).expect("reasoning menu descriptor").as_str();
        let labels = anchor.get(2).expect("reasoning menu labels").as_str();
        let rendered = anchor
            .get(4)
            .expect("reasoning menu rendered label")
            .as_str();
        let window_start = anchor.get(0).expect("reasoning menu anchor").end();
        let mut window_end = window_start.saturating_add(512).min(text.len());
        while window_end > window_start && !text.is_char_boundary(window_end) {
            window_end -= 1;
        }
        let window_text = &text[window_start..window_end];
        let selected_before_regex = Regex::new(&format!(
            r"{}=\(0,{IDENT}\.jsx\)\({IDENT},\{{\.\.\.{}\}}\)",
            regex::escape(rendered),
            regex::escape(descriptor),
        ))
        .expect("valid reasoning menu selected label regex");
        let selected_after_regex = Regex::new(&format!(
            r"{}={}\.defaultMessage",
            regex::escape(rendered),
            regex::escape(descriptor),
        ))
        .expect("valid patched reasoning menu selected label regex");
        let menu_before_regex = Regex::new(&format!(
            r"children:\(0,{IDENT}\.jsx\)\({IDENT},\{{\.\.\.{}\[({IDENT})\]\}}\)",
            regex::escape(labels),
        ))
        .expect("valid reasoning menu item label regex");
        let menu_after_regex = Regex::new(&format!(
            r"children:{}\[({IDENT})\]\.defaultMessage",
            regex::escape(labels),
        ))
        .expect("valid patched reasoning menu item label regex");

        let selected_before = selected_before_regex
            .find_iter(window_text)
            .map(|matched| {
                (
                    window_start + matched.start(),
                    window_start + matched.end(),
                    matched.as_str().to_string(),
                )
            })
            .collect::<Vec<_>>();
        let selected_after_count = selected_after_regex.find_iter(window_text).count();
        let menu_before = menu_before_regex
            .captures_iter(text)
            .map(|captures| {
                let whole = captures.get(0).expect("reasoning menu item label");
                (
                    whole.start(),
                    whole.end(),
                    whole.as_str().to_string(),
                    captures
                        .get(1)
                        .expect("reasoning menu effort binding")
                        .as_str()
                        .to_string(),
                )
            })
            .collect::<Vec<_>>();
        let menu_after_count = menu_after_regex.find_iter(text).count();
        if selected_before.len() + selected_after_count == 1
            && menu_before.len() + menu_after_count == 2
        {
            candidates.push(ReasoningMenuVisibleLabelCandidate {
                descriptor: descriptor.to_string(),
                labels: labels.to_string(),
                rendered: rendered.to_string(),
                selected_before,
                selected_after_count,
                menu_before,
                menu_after_count,
            });
        }
    }

    if candidates.is_empty() {
        return TextPatch::state(PatchDisposition::Missing);
    }
    if candidates.len() > 1 {
        return TextPatch::state(PatchDisposition::Ambiguous);
    }
    let candidate = candidates.pop().expect("single reasoning menu candidate");
    if !candidate.menu_before.is_empty() && candidate.menu_after_count > 0 {
        return TextPatch::state(PatchDisposition::Ambiguous);
    }
    if candidate.selected_after_count == 1 && candidate.menu_after_count == 2 {
        return TextPatch::patched();
    }

    let mut replacements = Vec::new();
    if let Some((start, end, before)) = candidate.selected_before.first() {
        let replacement = format!(
            "{}={}.defaultMessage",
            candidate.rendered, candidate.descriptor
        );
        let Some(padded) = pad_same_size_replacement(before, &replacement) else {
            return TextPatch::state(PatchDisposition::UnsafeSizeChange);
        };
        replacements.push((*start, *end, padded));
    }
    for (start, end, before, effort) in &candidate.menu_before {
        let replacement = format!("children:{}[{}].defaultMessage", candidate.labels, effort);
        let Some(padded) = pad_same_size_replacement(before, &replacement) else {
            return TextPatch::state(PatchDisposition::UnsafeSizeChange);
        };
        replacements.push((*start, *end, padded));
    }
    replacements.sort_by(|left, right| right.0.cmp(&left.0));
    let mut patched = text.to_string();
    for (start, end, replacement) in replacements {
        patched = replace_range(&patched, start, end, &replacement);
    }
    TextPatch::patchable(patched)
}

fn has_complete_power_selections(text: &str) -> bool {
    let current_filter_count = count_occurrences(text, CURRENT_LEGACY_POWER_SELECTION_FILTER)
        + count_occurrences(text, CURRENT_FALLBACK_POWER_SELECTION_FILTER)
        + count_occurrences(text, CURRENT_SPLIT_POWER_SELECTION_FILTER);
    validate_current_power_selections(text).state == PatchDisposition::Patched
        && text.contains("terra,sol,luna")
        && text.contains("gpt-5.6-${t}:${e}")
        && text.contains("t===`luna`&&e===`ultra`?`max`:e")
        && text.contains("low,medium,high,xhigh,max,ultra")
        && current_filter_count == 1
        && replace_collapsed_power_selection_label(text).state == PatchDisposition::Patched
        && replace_reasoning_menu_visible_labels(text).state == PatchDisposition::Patched
        && count_occurrences(text, "`cdpw`") == 1
}

fn replace_power_selections(text: &str) -> TextPatch {
    let selections = if text.contains(POWER_SELECTIONS_PATCH_MARKER) {
        validate_current_power_selections(text)
    } else {
        let previous_markers = PREVIOUS_POWER_SELECTIONS_PATCH_MARKERS
            .iter()
            .copied()
            .filter(|marker| text.contains(marker))
            .collect::<Vec<_>>();
        if !previous_markers.is_empty() {
            if previous_markers.len() != 1 {
                return TextPatch::state(PatchDisposition::Ambiguous);
            }
            replace_marked_power_selections(text, previous_markers[0])
        } else if text.contains(LEGACY_SOL_POWER_SELECTIONS_PATCH_MARKER) {
            replace_marked_power_selections(text, LEGACY_SOL_POWER_SELECTIONS_PATCH_MARKER)
        } else {
            let unified_count = count_occurrences(text, LEGACY_SOL_POWER_SELECTIONS);
            let split_count = if unified_count == 0 {
                count_occurrences(text, LEGACY_SPLIT_POWER_SELECTIONS)
            } else {
                0
            };
            if unified_count > 1 || split_count > 1 {
                return TextPatch::state(PatchDisposition::Ambiguous);
            }
            if unified_count == 0 && split_count == 0 {
                return TextPatch::state(PatchDisposition::Missing);
            }
            let legacy_selection = if unified_count == 1 {
                LEGACY_SOL_POWER_SELECTIONS
            } else {
                LEGACY_SPLIT_POWER_SELECTIONS
            };
            let selections_offset = text
                .find(legacy_selection)
                .expect("counted legacy selections must have an offset");
            let assignment_regex = Regex::new(&format!(r"({IDENT})=\[[ \t]*$"))
                .expect("valid legacy selection assignment regex");
            let Some(assignment) = assignment_regex.captures(&text[..selections_offset]) else {
                return TextPatch::state(PatchDisposition::Missing);
            };
            let full_assignment = assignment.get(0).expect("full assignment");
            let before_offset = full_assignment.start();
            let closing_bracket_offset = selections_offset + legacy_selection.len();
            if text.as_bytes().get(closing_bracket_offset) != Some(&b']') {
                return TextPatch::state(PatchDisposition::Missing);
            }
            let before_end = closing_bracket_offset + 1;
            let before = &text[before_offset..before_end];
            let after = power_selections_replacement(
                assignment.get(1).expect("assignment variable").as_str(),
            );
            let Some(replacement) = pad_same_size_replacement(before, &after) else {
                return TextPatch::state(PatchDisposition::UnsafeSizeChange);
            };
            TextPatch::patchable(replace_range(text, before_offset, before_end, &replacement))
        }
    };

    if !matches!(
        selections.state,
        PatchDisposition::Patched | PatchDisposition::Patchable
    ) {
        return selections;
    }
    let selections_text = selections.text.as_deref().unwrap_or(text);
    let filter = replace_power_selection_filter(selections_text);
    if !matches!(
        filter.state,
        PatchDisposition::Patched | PatchDisposition::Patchable
    ) {
        return filter;
    }
    let filter_text = filter.text.as_deref().unwrap_or(selections_text);
    let menu_width = replace_power_picker_menu_width(filter_text);
    if !matches!(
        menu_width.state,
        PatchDisposition::Patched | PatchDisposition::Patchable
    ) {
        return menu_width;
    }
    let menu_width_text = menu_width.text.as_deref().unwrap_or(filter_text);
    let collapsed_label = replace_collapsed_power_selection_label(menu_width_text);
    if !matches!(
        collapsed_label.state,
        PatchDisposition::Patched | PatchDisposition::Patchable
    ) {
        return collapsed_label;
    }
    let collapsed_label_text = collapsed_label.text.as_deref().unwrap_or(menu_width_text);
    let visible_menu_labels = replace_reasoning_menu_visible_labels(collapsed_label_text);
    if !matches!(
        visible_menu_labels.state,
        PatchDisposition::Patched | PatchDisposition::Patchable
    ) {
        return visible_menu_labels;
    }
    let patched = visible_menu_labels
        .text
        .as_deref()
        .unwrap_or(collapsed_label_text);
    if !has_complete_power_selections(patched) {
        return TextPatch::state(PatchDisposition::Missing);
    }
    if selections.state == PatchDisposition::Patched
        && filter.state == PatchDisposition::Patched
        && menu_width.state == PatchDisposition::Patched
        && collapsed_label.state == PatchDisposition::Patched
        && visible_menu_labels.state == PatchDisposition::Patched
    {
        TextPatch::patched()
    } else {
        TextPatch::patchable(patched.to_string())
    }
}

#[derive(Debug)]
struct PowerSelectionScopeCall {
    alias: String,
    index: usize,
    length: usize,
    model: String,
    result: String,
    selected_model: Option<String>,
}

fn find_power_selection_scope_call(
    text: &str,
) -> Option<Result<PowerSelectionScopeCall, PatchDisposition>> {
    if !text.contains("powerSelections:") {
        return None;
    }
    let import_regex =
        Regex::new(r#"import\{([^}]*)\}from"\./model-and-reasoning-dropdown-[^"]+\.js";"#)
            .expect("valid Power scope import regex");
    let import_match = import_regex.captures(text)?;
    let imported = import_match.get(1).expect("import bindings").as_str();
    let export_regex = Regex::new(&format!(
        r"(?:^|,)\s*{}\s+as\s+({IDENT})\b",
        regex::escape(POWER_SELECTIONS_EXPORT_NAME),
    ))
    .expect("valid Power scope export regex");
    let export_match = export_regex.captures(imported)?;
    let alias = export_match
        .get(1)
        .expect("Power scope alias")
        .as_str()
        .to_string();
    let calls_regex = Regex::new(&format!(
        r"({IDENT})={}\(({IDENT})(?:,({IDENT}))?\)",
        regex::escape(&alias),
    ))
    .expect("valid Power scope call regex");
    let calls = calls_regex.captures_iter(text).collect::<Vec<_>>();
    if calls.is_empty() {
        return Some(Err(PatchDisposition::Missing));
    }
    if calls.len() > 1 {
        return Some(Err(PatchDisposition::Ambiguous));
    }
    let call = &calls[0];
    let whole = call.get(0).expect("Power scope call");
    Some(Ok(PowerSelectionScopeCall {
        alias,
        index: whole.start(),
        length: whole.end() - whole.start(),
        model: call.get(2).expect("Power scope model").as_str().to_string(),
        result: call
            .get(1)
            .expect("Power scope result")
            .as_str()
            .to_string(),
        selected_model: call.get(3).map(|value| value.as_str().to_string()),
    }))
}

fn is_power_selection_scope_candidate(text: &str) -> bool {
    find_power_selection_scope_call(text).is_some()
}

fn utf16_window_start_byte(text: &str, end: usize, max_code_units: usize) -> usize {
    let mut end = end.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut consumed = 0_usize;
    for (start, character) in text[..end].char_indices().rev() {
        let width = character.len_utf16();
        if consumed.saturating_add(width) > max_code_units {
            // JS can slice through a surrogate pair. Regexes below only need the
            // suffix after that boundary, so omit the split scalar instead.
            return start + character.len_utf8();
        }
        consumed += width;
        if consumed == max_code_units {
            return start;
        }
    }
    0
}

fn fit_power_scope_patch_to_original_byte_length(
    text: String,
    expected_length: usize,
) -> Option<String> {
    let fitted = if text.as_bytes().len() > expected_length {
        text.replacen(";\n//# sourceMappingURL=", "//# sourceMappingURL=", 1)
    } else {
        text
    };
    pad_to_byte_length(fitted, expected_length)
}

fn replace_power_selection_scope_call(text: &str) -> TextPatch {
    let Some(call) = find_power_selection_scope_call(text) else {
        return TextPatch::state(PatchDisposition::Missing);
    };
    let call = match call {
        Ok(call) => call,
        Err(state) => return TextPatch::state(state),
    };
    if call.selected_model.is_some() {
        return TextPatch::patched();
    }

    let context_start = utf16_window_start_byte(text, call.index, 1024);
    let context_end = call.index + call.length;
    let context = &text[context_start..context_end];
    let neighbor_regex = Regex::new(&format!(
        r"={}\({},({IDENT})\),{}={}\({}\)$",
        IDENT,
        regex::escape(&call.model),
        regex::escape(&call.result),
        regex::escape(&call.alias),
        regex::escape(&call.model),
    ))
    .expect("valid neighboring selected-model regex");
    let selected_from_neighbor = neighbor_regex
        .captures(context)
        .and_then(|captures| captures.get(1))
        .map(|capture| capture.as_str().to_string());
    let before_start = utf16_window_start_byte(text, call.index, 4096);
    let model_settings_regex =
        Regex::new(&format!(r"({IDENT})={IDENT}\.model")).expect("valid model-settings regex");
    let selected_from_settings = model_settings_regex
        .captures_iter(&text[before_start..call.index])
        .filter_map(|captures| captures.get(1).map(|capture| capture.as_str().to_string()))
        .last();
    let Some(selected_model) = selected_from_neighbor.or(selected_from_settings) else {
        return TextPatch::state(PatchDisposition::Missing);
    };

    let original_length = text.as_bytes().len();
    let replacement = format!(
        "{}={}({},{})",
        call.result, call.alias, call.model, selected_model
    );
    let patched = replace_range(text, call.index, call.index + call.length, &replacement);
    let Some(fitted) = fit_power_scope_patch_to_original_byte_length(patched, original_length)
    else {
        return TextPatch::state(PatchDisposition::UnsafeSizeChange);
    };
    TextPatch::patchable(fitted)
}

fn is_zh_cn_asset(entry_path: &str) -> bool {
    entry_path.starts_with("webview/assets/zh-CN-") && entry_path.ends_with(".js")
}

fn replace_exact_same_size_label(text: &str, before: &str, after: &str) -> TextPatch {
    let before_count = count_occurrences(text, before);
    let after_count = count_occurrences(text, after);
    if before_count > 0 && after_count > 0 {
        return TextPatch::state(PatchDisposition::Ambiguous);
    }
    if after_count == 1 {
        return TextPatch::patched();
    }
    if after_count > 1 || before_count > 1 {
        return TextPatch::state(PatchDisposition::Ambiguous);
    }
    if before_count == 0 {
        return TextPatch::state(PatchDisposition::Missing);
    }
    let Some(replacement) = pad_same_size_replacement(before, after) else {
        return TextPatch::state(PatchDisposition::UnsafeSizeChange);
    };
    TextPatch::patchable(text.replacen(before, &replacement, 1))
}

fn replace_zh_cn_reasoning_labels(entry_path: &str, text: &str) -> TextPatch {
    if !is_zh_cn_asset(entry_path) {
        return TextPatch::state(PatchDisposition::Missing);
    }

    let mut patched_text = text.to_string();
    let mut changed = false;
    for (before, after) in [
        (ZH_CN_MAX_LABEL_BEFORE, ZH_CN_MAX_LABEL_AFTER),
        (ZH_CN_ULTRA_LABEL_BEFORE, ZH_CN_ULTRA_LABEL_AFTER),
    ] {
        let result = replace_exact_same_size_label(&patched_text, before, after);
        match result.state {
            PatchDisposition::Patchable => {
                patched_text = result.text.expect("patchable label replacement");
                changed = true;
            }
            PatchDisposition::Patched => {}
            _ => return TextPatch::state(result.state),
        }
    }

    if changed {
        TextPatch::patchable(patched_text)
    } else {
        TextPatch::patched()
    }
}

fn is_composer_collapsed_reasoning_label_candidate(entry_path: &str, text: &str) -> bool {
    entry_path.starts_with("webview/assets/")
        && entry_path.ends_with(".js")
        && text.contains("expandedContentWidth")
        && text.contains("labelCandidates")
        && text.contains("WorkTriggerEffortLabel")
}

fn replace_composer_collapsed_reasoning_label(text: &str) -> TextPatch {
    let before_regex = Regex::new(
        r"([A-Za-z_$][A-Za-z0-9_$]*)=([A-Za-z_$][A-Za-z0-9_$]*)=>\(\{\.\.\.([A-Za-z_$][A-Za-z0-9_$]*),effortLabel:([A-Za-z_$][A-Za-z0-9_$]*)\.formatMessage\(([A-Za-z_$][A-Za-z0-9_$]*)\[([A-Za-z_$][A-Za-z0-9_$]*)\.reasoningEffort\]\)\}\)",
    )
    .expect("valid original composer reasoning label regex");
    let after_regex = Regex::new(
        r"([A-Za-z_$][A-Za-z0-9_$]*)=([A-Za-z_$][A-Za-z0-9_$]*)=>\(\{\.\.\.([A-Za-z_$][A-Za-z0-9_$]*),effortLabel:([A-Za-z_$][A-Za-z0-9_$]*)\[([A-Za-z_$][A-Za-z0-9_$]*)\.reasoningEffort\]\.defaultMessage\}\)",
    )
    .expect("valid patched composer reasoning label regex");
    let before_matches = before_regex
        .captures_iter(text)
        .filter(|captures| {
            captures.get(2).map(|value| value.as_str())
                == captures.get(3).map(|value| value.as_str())
                && captures.get(2).map(|value| value.as_str())
                    == captures.get(6).map(|value| value.as_str())
        })
        .collect::<Vec<_>>();
    let after_matches = after_regex
        .captures_iter(text)
        .filter(|captures| {
            captures.get(2).map(|value| value.as_str())
                == captures.get(3).map(|value| value.as_str())
                && captures.get(2).map(|value| value.as_str())
                    == captures.get(5).map(|value| value.as_str())
        })
        .collect::<Vec<_>>();

    if before_matches.len() > 1
        || after_matches.len() > 1
        || (!before_matches.is_empty() && !after_matches.is_empty())
    {
        return TextPatch::state(PatchDisposition::Ambiguous);
    }
    if !after_matches.is_empty() {
        return TextPatch::patched();
    }
    let Some(original) = before_matches.first() else {
        return TextPatch::state(PatchDisposition::Missing);
    };

    let whole = original.get(0).expect("whole composer reasoning formatter");
    let binding = original
        .get(1)
        .expect("composer formatter binding")
        .as_str();
    let argument = original
        .get(2)
        .expect("composer formatter argument")
        .as_str();
    let labels = original.get(5).expect("reasoning labels binding").as_str();
    let replacement = format!(
        "{binding}={argument}=>({{...{argument},effortLabel:{labels}[{argument}.reasoningEffort].defaultMessage}})"
    );
    let Some(padded) = pad_same_size_replacement(whole.as_str(), &replacement) else {
        return TextPatch::state(PatchDisposition::UnsafeSizeChange);
    };
    TextPatch::patchable(replace_range(text, whole.start(), whole.end(), &padded))
}

fn replace_composer_visible_reasoning_label(text: &str) -> TextPatch {
    let anchor_regex = Regex::new(&format!(
        r"let ({IDENT})=({IDENT})===`ultra`,({IDENT})=({IDENT})\[({IDENT})\],({IDENT});"
    ))
    .expect("valid composer visible reasoning anchor regex");
    let anchors = anchor_regex
        .captures_iter(text)
        .filter(|captures| {
            captures.get(2).map(|value| value.as_str())
                == captures.get(5).map(|value| value.as_str())
        })
        .collect::<Vec<_>>();
    if anchors.is_empty() {
        return TextPatch::state(PatchDisposition::Missing);
    }
    if anchors.len() > 1 {
        return TextPatch::state(PatchDisposition::Ambiguous);
    }

    let anchor = &anchors[0];
    let descriptor = anchor.get(3).expect("composer descriptor binding").as_str();
    let rendered = anchor
        .get(6)
        .expect("composer rendered label binding")
        .as_str();
    let window_start = anchor.get(0).expect("composer visible anchor").end();
    let mut window_end = window_start.saturating_add(512).min(text.len());
    while window_end > window_start && !text.is_char_boundary(window_end) {
        window_end -= 1;
    }
    let window_text = &text[window_start..window_end];
    let before_regex = Regex::new(&format!(
        r"{}=\(0,{IDENT}\.jsx\)\({IDENT},\{{\.\.\.{}\}}\)",
        regex::escape(rendered),
        regex::escape(descriptor),
    ))
    .expect("valid composer visible reasoning formatter regex");
    let after_regex = Regex::new(&format!(
        r"{}={}\.defaultMessage",
        regex::escape(rendered),
        regex::escape(descriptor),
    ))
    .expect("valid patched composer visible reasoning formatter regex");
    let before_matches = before_regex.find_iter(window_text).collect::<Vec<_>>();
    let after_matches = after_regex.find_iter(window_text).collect::<Vec<_>>();
    if before_matches.len() > 1
        || after_matches.len() > 1
        || (!before_matches.is_empty() && !after_matches.is_empty())
    {
        return TextPatch::state(PatchDisposition::Ambiguous);
    }
    if !after_matches.is_empty() {
        return TextPatch::patched();
    }
    let Some(original) = before_matches.first() else {
        return TextPatch::state(PatchDisposition::Missing);
    };

    let replacement = format!("{rendered}={descriptor}.defaultMessage");
    let Some(padded) = pad_same_size_replacement(original.as_str(), &replacement) else {
        return TextPatch::state(PatchDisposition::UnsafeSizeChange);
    };
    TextPatch::patchable(replace_range(
        text,
        window_start + original.start(),
        window_start + original.end(),
        &padded,
    ))
}

fn replace_composer_reasoning_labels(text: &str) -> TextPatch {
    let mut patched_text = text.to_string();
    let mut changed = false;
    for patcher in [
        replace_composer_collapsed_reasoning_label as fn(&str) -> TextPatch,
        replace_composer_visible_reasoning_label,
    ] {
        let result = patcher(&patched_text);
        match result.state {
            PatchDisposition::Patchable => {
                patched_text = result.text.expect("patchable composer reasoning label");
                changed = true;
            }
            PatchDisposition::Patched => {}
            _ => return TextPatch::state(result.state),
        }
    }

    if changed {
        TextPatch::patchable(patched_text)
    } else {
        TextPatch::patched()
    }
}

fn is_reasoning_label_defaults_entry(entry_path: &str) -> bool {
    entry_path
        .strip_prefix("webview/assets/reasoning-effort-label-messages-")
        .is_some_and(|suffix| suffix.ends_with(".js"))
}

fn replace_reasoning_label_defaults(entry_path: &str, text: &str) -> TextPatch {
    if !is_reasoning_label_defaults_entry(entry_path) {
        return TextPatch::state(PatchDisposition::Missing);
    }

    let mut patched_text = text.to_string();
    let mut changed = false;
    for (before, after) in REASONING_LABEL_DEFAULT_REPLACEMENTS {
        let result = replace_exact_same_size_label(&patched_text, before, after);
        match result.state {
            PatchDisposition::Patchable => {
                patched_text = result.text.expect("patchable reasoning label default");
                changed = true;
            }
            PatchDisposition::Patched => {}
            _ => return TextPatch::state(result.state),
        }
    }

    if changed {
        TextPatch::patchable(patched_text)
    } else {
        TextPatch::patched()
    }
}

fn text_patch_to_entry(
    result: TextPatch,
    original_length: usize,
    patch_name: &'static str,
) -> EntryPatch {
    match result.state {
        PatchDisposition::Patched => EntryPatch::patched(patch_name),
        PatchDisposition::Patchable => {
            let bytes = result
                .text
                .expect("patchable text patch must retain replacement")
                .into_bytes();
            if bytes.len() != original_length {
                EntryPatch::state(PatchDisposition::UnsafeSizeChange)
            } else {
                EntryPatch::patchable(bytes, patch_name)
            }
        }
        state => EntryPatch::state(state),
    }
}

fn patch_entry_bytes(entry_path: &str, bytes: &[u8]) -> Result<EntryPatch, String> {
    let text = String::from_utf8_lossy(bytes);
    if is_stable_metadata_retry_asset(text.as_ref()) {
        let has_history_provider_filter = text.contains(HISTORY_PROVIDER_FILTER_BEFORE)
            || text.contains(HISTORY_PROVIDER_FILTER_AFTER);
        let patch = text_patch_to_entry(
            replace_stable_metadata_non_retry_guard(text.as_ref()),
            bytes.len(),
            "stable-metadata-git-retry",
        );
        return Ok(if has_history_provider_filter {
            patch.with_additional_patch_names(&["history-provider-filter"])
        } else {
            patch
        });
    }
    if is_composer_collapsed_reasoning_label_candidate(entry_path, text.as_ref()) {
        return Ok(text_patch_to_entry(
            replace_composer_reasoning_labels(text.as_ref()),
            bytes.len(),
            "composer-reasoning-labels",
        ));
    }
    if is_reasoning_label_defaults_entry(entry_path) {
        return Ok(text_patch_to_entry(
            replace_reasoning_label_defaults(entry_path, text.as_ref()),
            bytes.len(),
            "reasoning-effort-label-defaults",
        ));
    }
    if is_power_slider_js_entry(entry_path) {
        return Ok(text_patch_to_entry(
            replace_power_slider_component(text.as_ref())?,
            bytes.len(),
            "custom-model-picker-ui",
        ));
    }
    if is_power_slider_css_entry(entry_path) {
        return Ok(text_patch_to_entry(
            replace_power_slider_css(text.as_ref())?,
            bytes.len(),
            "custom-model-picker-ui",
        ));
    }

    if text.contains(POWER_SELECTIONS_PATCH_MARKER)
        || PREVIOUS_POWER_SELECTIONS_PATCH_MARKERS
            .iter()
            .any(|marker| text.contains(marker))
        || text.contains(LEGACY_SOL_POWER_SELECTIONS_PATCH_MARKER)
        || contains_power_picker_signature(text.as_ref())
    {
        return Ok(text_patch_to_entry(
            replace_power_selections(text.as_ref()),
            bytes.len(),
            "reasoning-effort-options",
        ));
    }

    if is_power_selection_scope_candidate(text.as_ref()) {
        return Ok(text_patch_to_entry(
            replace_power_selection_scope_call(text.as_ref()),
            bytes.len(),
            "reasoning-effort-current-model-scope",
        ));
    }

    if entry_path.starts_with("webview/assets/zh-CN-") {
        let label = replace_zh_cn_reasoning_labels(entry_path, text.as_ref());
        if label.state == PatchDisposition::Patched || label.state == PatchDisposition::Patchable {
            return Ok(text_patch_to_entry(
                label,
                bytes.len(),
                "reasoning-effort-labels-zh-cn",
            ));
        }
        if label.state == PatchDisposition::Ambiguous {
            return Ok(EntryPatch::state(label.state));
        }
    }

    if text.contains("availableModels")
        && text.contains("useHiddenModels")
        && text.contains("enabledReasoningEfforts")
    {
        return Ok(text_patch_to_entry(
            replace_model_list_filter(text.as_ref()),
            bytes.len(),
            "model-picker",
        ));
    }
    if text.contains("availableModels")
        && text.contains("useHiddenModels")
        && text.contains("amazonBedrock")
    {
        return Ok(text_patch_to_entry(
            replace_model_picker_gate(text.as_ref()),
            bytes.len(),
            "model-picker",
        ));
    }

    if text.contains(HISTORY_PROVIDER_FILTER_AFTER) {
        return Ok(EntryPatch::patched("history-provider-filter"));
    }
    if text.contains(HISTORY_PROVIDER_FILTER_BEFORE) {
        return Ok(text_patch_to_entry(
            TextPatch::patchable(text.replace(
                HISTORY_PROVIDER_FILTER_BEFORE,
                HISTORY_PROVIDER_FILTER_AFTER,
            )),
            bytes.len(),
            "history-provider-filter",
        ));
    }
    Ok(EntryPatch::state(PatchDisposition::Missing))
}

fn contains_model_picker_signature(text: &str) -> bool {
    text.contains("availableModels") && text.contains("useHiddenModels")
}

fn contains_model_picker_gate_signature(text: &str) -> bool {
    contains_model_picker_signature(text)
        && (text.contains("enabledReasoningEfforts") || text.contains("amazonBedrock"))
}

fn has_applied_model_picker_gate(text: &str) -> bool {
    [
        format!(r"[,;]{IDENT}=0&&{IDENT}!==`amazonBedrock`[,;]"),
        format!(r#"[,;]{IDENT}=0&&{IDENT}!=="amazonBedrock"[,;]"#),
    ]
    .iter()
    .any(|pattern| {
        Regex::new(pattern)
            .expect("valid applied model-gate regex")
            .is_match(text)
    })
}

fn has_invalid_power_selection_closing(text: &str) -> bool {
    let mut cursor = 0;
    while let Some(relative) = text[cursor..].find(']') {
        let first = cursor + relative;
        let mut next = first + 1;
        while next < text.len() {
            let character = text[next..].chars().next().expect("valid char boundary");
            if !character.is_whitespace() {
                break;
            }
            next += character.len_utf8();
        }
        if text[next..].starts_with("]}));") {
            return true;
        }
        cursor = first + 1;
    }
    false
}

fn verify_entry_patch(entry_path: &str, bytes: &[u8]) -> Result<bool, String> {
    let text = String::from_utf8_lossy(bytes);
    if is_stable_metadata_retry_asset(text.as_ref()) {
        return Ok(replace_stable_metadata_non_retry_guard(text.as_ref()).state
            == PatchDisposition::Patched);
    }
    if is_composer_collapsed_reasoning_label_candidate(entry_path, text.as_ref()) {
        return Ok(
            replace_composer_reasoning_labels(text.as_ref()).state == PatchDisposition::Patched
        );
    }
    if is_reasoning_label_defaults_entry(entry_path) {
        return Ok(
            replace_reasoning_label_defaults(entry_path, text.as_ref()).state
                == PatchDisposition::Patched,
        );
    }
    if is_power_slider_js_entry(entry_path) {
        return Ok(
            replace_power_slider_component(text.as_ref())?.state == PatchDisposition::Patched
        );
    }
    if is_power_slider_css_entry(entry_path) {
        return Ok(replace_power_slider_css(text.as_ref())?.state == PatchDisposition::Patched);
    }
    if text.contains(POWER_SELECTIONS_PATCH_MARKER) {
        return Ok(has_complete_power_selections(text.as_ref())
            && !has_invalid_power_selection_closing(text.as_ref()));
    }
    if is_power_selection_scope_candidate(text.as_ref()) {
        return Ok(matches!(
            find_power_selection_scope_call(text.as_ref()),
            Some(Ok(PowerSelectionScopeCall {
                selected_model: Some(_),
                ..
            }))
        ));
    }
    if entry_path.starts_with("webview/assets/zh-CN-")
        && text.contains(ZH_CN_MAX_LABEL_AFTER)
        && text.contains(ZH_CN_ULTRA_LABEL_AFTER)
    {
        return Ok(true);
    }
    if text.contains("availableModels")
        && text.contains("useHiddenModels")
        && text.contains("enabledReasoningEfforts")
    {
        return Ok((has_applied_model_picker_gate(text.as_ref())
            || is_compacted_model_picker_gate(text.as_ref()))
            && !max_reasoning_matches(text.as_ref(), true).is_empty());
    }
    if text.contains("availableModels")
        && text.contains("useHiddenModels")
        && text.contains("amazonBedrock")
    {
        return Ok(has_applied_model_picker_gate(text.as_ref()));
    }
    if text.contains(HISTORY_PROVIDER_FILTER_BEFORE) {
        return Ok(false);
    }
    Ok(true)
}

fn is_preferred_model_picker_entry(entry_path: &str) -> bool {
    entry_path
        .strip_prefix("webview/assets/model-list-filter-")
        .is_some_and(|suffix| suffix.ends_with(".js"))
}

fn candidate_asar_files(files: &[AsarFile]) -> Vec<AsarFile> {
    let preferred = files
        .iter()
        .filter(|file| is_preferred_model_picker_entry(&file.path))
        .cloned()
        .collect::<Vec<_>>();
    if !preferred.is_empty() {
        return preferred;
    }
    files
        .iter()
        .filter(|file| {
            file.path.starts_with("webview/assets/")
                && file.path.ends_with(".js")
                && file.size <= 20_000
        })
        .cloned()
        .collect()
}

fn patch_candidate_asar_files(files: &[AsarFile]) -> Vec<AsarFile> {
    let mut candidates = candidate_asar_files(files);
    let mut seen = candidates
        .iter()
        .map(|file| file.path.clone())
        .collect::<BTreeSet<_>>();
    for file in files {
        if (is_power_slider_js_entry(&file.path) || is_power_slider_css_entry(&file.path))
            && seen.insert(file.path.clone())
        {
            candidates.push(file.clone());
        }
    }
    for file in files {
        if file.path.ends_with(".js") && seen.insert(file.path.clone()) {
            candidates.push(file.clone());
        }
    }
    candidates
}

fn append_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
    }
}

fn combined_patch_names(already: &[String], patched: &[String]) -> Vec<String> {
    let mut names = Vec::new();
    for name in already.iter().chain(patched.iter()) {
        append_unique(&mut names, name);
    }
    names
}

fn js_iso_year(year: i32) -> String {
    if (0..=9_999).contains(&year) {
        format!("{year:04}")
    } else {
        let sign = if year < 0 { '-' } else { '+' };
        format!("{sign}{:06}", year.unsigned_abs())
    }
}

fn format_js_iso_timestamp(value: time::OffsetDateTime) -> String {
    let value = value.to_offset(time::UtcOffset::UTC);
    format!(
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        js_iso_year(value.year()),
        value.month() as u8,
        value.day(),
        value.hour(),
        value.minute(),
        value.second(),
        value.nanosecond() / 1_000_000,
    )
}

fn rfc3339_now() -> Result<String, String> {
    Ok(format_js_iso_timestamp(time::OffsetDateTime::now_utc()))
}

fn temporary_sibling_path(parent: &Path, file_name: &str, kind: &str) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(
        ".{file_name}.{kind}-{}-{sequence}",
        std::process::id()
    ))
}

fn create_backup(asar_path: &Path, backup_dir: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(backup_dir).map_err(|error| {
        format!(
            "创建 app.asar 备份目录失败 {}: {error}",
            backup_dir.display()
        )
    })?;
    let stamp = rfc3339_now()?.replace(':', "-").replace('.', "-");
    for attempt in 0..32_u64 {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!("-{attempt}")
        };
        let backup_path = backup_dir.join(format!("app.asar.controlled.{stamp}{suffix}.bak"));
        if backup_path.exists() {
            continue;
        }
        fs::copy(asar_path, &backup_path).map_err(|error| {
            format!(
                "备份 app.asar 失败 {} -> {}: {error}",
                asar_path.display(),
                backup_path.display()
            )
        })?;
        return Ok(backup_path);
    }
    Err(format!(
        "无法为 app.asar 创建唯一备份文件: {}",
        backup_dir.display()
    ))
}

#[cfg(target_os = "windows")]
fn replace_patch_state_file(
    source: &Path,
    destination: &Path,
    backup: &Path,
) -> Result<(), String> {
    use std::iter;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{ReplaceFileW, REPLACEFILE_WRITE_THROUGH};

    if !destination.exists() {
        return fs::rename(source, destination).map_err(|error| {
            format!(
                "写入 patch 状态失败 {} -> {}: {error}",
                source.display(),
                destination.display()
            )
        });
    }
    let destination_wide = destination
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    let source_wide = source
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    let backup_wide = backup
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    unsafe {
        ReplaceFileW(
            PCWSTR(destination_wide.as_ptr()),
            PCWSTR(source_wide.as_ptr()),
            PCWSTR(backup_wide.as_ptr()),
            REPLACEFILE_WRITE_THROUGH,
            None,
            None,
        )
    }
    .map_err(|error| {
        format!(
            "原子替换 patch 状态失败 {} -> {}: {error}",
            source.display(),
            destination.display()
        )
    })
}

#[cfg(not(target_os = "windows"))]
fn replace_patch_state_file(
    source: &Path,
    destination: &Path,
    _backup: &Path,
) -> Result<(), String> {
    fs::rename(source, destination).map_err(|error| {
        format!(
            "原子替换 patch 状态失败 {} -> {}: {error}",
            source.display(),
            destination.display()
        )
    })
}

fn write_bytes_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("无法解析 patch 状态目录 {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建 patch 状态目录失败 {}: {error}", parent.display()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("model-picker-patch-state.json");
    let backup_path = temporary_sibling_path(parent, file_name, "previous");
    let mut temporary_path = None;

    let result = (|| -> Result<(), String> {
        let mut temporary = None;
        for _ in 0..16 {
            let candidate = temporary_sibling_path(parent, file_name, "tmp");
            match OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&candidate)
            {
                Ok(file) => {
                    temporary_path = Some(candidate);
                    temporary = Some(file);
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(format!(
                        "创建临时 patch 状态失败 {}: {error}",
                        candidate.display()
                    ));
                }
            }
        }
        let temporary_path = temporary_path
            .as_ref()
            .ok_or_else(|| format!("无法创建唯一临时 patch 状态文件: {}", parent.display()))?;
        let mut temporary = temporary.expect("temporary path has an open file");
        temporary.write_all(bytes).map_err(|error| {
            format!(
                "写入临时 patch 状态失败 {}: {error}",
                temporary_path.display()
            )
        })?;
        temporary.sync_all().map_err(|error| {
            format!(
                "刷新临时 patch 状态失败 {}: {error}",
                temporary_path.display()
            )
        })?;
        drop(temporary);

        let had_previous = path.exists();
        replace_patch_state_file(&temporary_path, path, &backup_path)?;
        if had_previous {
            if let Err(error) = fs::remove_file(&backup_path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    log::warn!(
                        "清理旧 patch 状态备份失败 {}: {error}",
                        backup_path.display()
                    );
                }
            }
        }
        Ok(())
    })();

    if result.is_err() {
        if let Some(temporary_path) = temporary_path {
            let _ = fs::remove_file(temporary_path);
        }
    }
    result
}

fn write_patch_state(path: &Path, state: &Value) -> Result<(), String> {
    let mut serialized = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("序列化 patch 状态失败: {error}"))?;
    serialized.push(b'\n');
    write_bytes_atomically(path, &serialized)
}

fn restore_patch_state(path: &Path, previous: Option<&[u8]>) -> Result<(), String> {
    match previous {
        None => {
            if let Err(error) = fs::remove_file(path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    return Err(format!("移除 patch 状态失败 {}: {error}", path.display()));
                }
            }
            Ok(())
        }
        Some(previous) => {
            if fs::read(path).ok().as_deref() == Some(previous) {
                return Ok(());
            }
            write_bytes_atomically(path, previous)
        }
    }
}

fn optional_trimmed_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn path_string(value: Option<&Path>) -> String {
    value
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn patch_state_value(
    request: &PatchRequest,
    status: &str,
    patch_version: &str,
    source_asar_hash: &str,
    source_codex_version: Option<&str>,
    patched_asar_hash: &str,
    patch_names: &[String],
    backup_path: Option<&Path>,
    patched_files: Option<&[String]>,
    patched_header_hash: Option<&str>,
) -> Result<Value, String> {
    let mut state = Map::new();
    state.insert("status".to_string(), Value::String(status.to_string()));
    state.insert(
        "patchVersion".to_string(),
        Value::String(patch_version.to_string()),
    );
    state.insert("mode".to_string(), Value::String("controlled".to_string()));
    state.insert(
        "appAsarPath".to_string(),
        Value::String(request.app_asar_path.to_string_lossy().to_string()),
    );
    state.insert(
        "controlledAppRoot".to_string(),
        Value::String(path_string(request.controlled_app_root.as_deref())),
    );
    state.insert(
        "launchPath".to_string(),
        Value::String(path_string(request.launch_path.as_deref())),
    );
    if let Some(backup_path) = backup_path {
        state.insert(
            "backupPath".to_string(),
            Value::String(backup_path.to_string_lossy().to_string()),
        );
    }
    state.insert(
        "sourceAsarHash".to_string(),
        Value::String(source_asar_hash.to_string()),
    );
    state.insert(
        "sourceCodexVersion".to_string(),
        source_codex_version
            .map(|value| Value::String(value.to_string()))
            .unwrap_or(Value::Null),
    );
    state.insert(
        "patchedAsarHash".to_string(),
        Value::String(patched_asar_hash.to_string()),
    );
    state.insert(
        "patchNames".to_string(),
        Value::Array(patch_names.iter().cloned().map(Value::String).collect()),
    );
    if let Some(patched_files) = patched_files {
        state.insert(
            "patchedFiles".to_string(),
            Value::Array(patched_files.iter().cloned().map(Value::String).collect()),
        );
    }
    if let Some(patched_header_hash) = patched_header_hash {
        state.insert(
            "patchedHeaderHash".to_string(),
            Value::String(patched_header_hash.to_string()),
        );
    }
    state.insert("patchedAt".to_string(), Value::String(rfc3339_now()?));
    Ok(Value::Object(state))
}

/// Patches only the supplied controlled `app.asar` target. The caller owns source,
/// WindowsApps, controlled-path, and marker validation; this module never receives
/// or writes a source-install path.
pub(crate) fn patch_controlled_app(request: &PatchRequest) -> Result<Value, String> {
    if !request.app_asar_path.is_file() {
        return Err(format!(
            "缺少受控 app.asar: {}",
            request.app_asar_path.display()
        ));
    }
    let before_bytes = fs::read(&request.app_asar_path).map_err(|error| {
        format!(
            "读取 app.asar 失败 {}: {error}",
            request.app_asar_path.display()
        )
    })?;
    let target_asar_hash_before_patch = sha256_hex(&before_bytes);
    let source_asar_hash = optional_trimmed_string(request.source_asar_hash.as_deref())
        .unwrap_or_else(|| target_asar_hash_before_patch.clone());
    let source_codex_version = optional_trimmed_string(request.source_codex_version.as_deref());
    let patch_version = optional_trimmed_string(Some(&request.patch_version))
        .unwrap_or_else(|| DEFAULT_PATCH_VERSION.to_string());

    let mut asar = read_asar_header(&request.app_asar_path)?;
    let files = patch_candidate_asar_files(&list_asar_files(&asar.header)?);
    let mut targets = Vec::new();
    let mut patch_names = Vec::new();
    let mut already_patch_names = Vec::new();
    let mut saw_model_picker = false;
    let mut saw_power_picker = false;
    let mut saw_power_selection_scope = false;
    let mut saw_power_slider_js = false;
    let mut saw_power_slider_css = false;
    let mut saw_composer_collapsed_reasoning_label = false;
    let mut saw_reasoning_label_defaults = false;
    let mut saw_stable_metadata_retry = false;

    for file in files {
        let (offset, bytes) = read_asar_entry(&request.app_asar_path, asar.files_offset, &file)?;
        let text = String::from_utf8_lossy(&bytes);
        let result = patch_entry_bytes(&file.path, &bytes)?;
        let is_power_picker = text.contains(POWER_SELECTIONS_PATCH_MARKER)
            || PREVIOUS_POWER_SELECTIONS_PATCH_MARKERS
                .iter()
                .any(|marker| text.contains(marker))
            || text.contains(LEGACY_SOL_POWER_SELECTIONS_PATCH_MARKER)
            || contains_power_picker_signature(text.as_ref());
        let is_power_selection_scope = is_power_selection_scope_candidate(text.as_ref());
        let is_power_slider_js = is_power_slider_js_entry(&file.path);
        let is_power_slider_css = is_power_slider_css_entry(&file.path);
        let is_composer_collapsed_reasoning_label =
            is_composer_collapsed_reasoning_label_candidate(&file.path, text.as_ref());
        let is_reasoning_label_defaults = is_reasoning_label_defaults_entry(&file.path);
        let is_stable_metadata_retry = is_stable_metadata_retry_asset(text.as_ref());
        let is_preferred_model_picker = is_preferred_model_picker_entry(&file.path);
        saw_power_picker |= is_power_picker;
        saw_power_selection_scope |= is_power_selection_scope;
        saw_power_slider_js |= is_power_slider_js;
        saw_power_slider_css |= is_power_slider_css;
        saw_composer_collapsed_reasoning_label |= is_composer_collapsed_reasoning_label;
        saw_reasoning_label_defaults |= is_reasoning_label_defaults;
        saw_stable_metadata_retry |= is_stable_metadata_retry;

        if is_stable_metadata_retry
            && (result.state.is_failure() || result.patch_name != Some("stable-metadata-git-retry"))
        {
            return Err(format!(
                "Stable metadata Git retry patch failed for {}: {}",
                file.path,
                result.state.as_str()
            ));
        }

        if result.patch_name == Some("model-picker")
            || (is_preferred_model_picker && contains_model_picker_gate_signature(text.as_ref()))
        {
            saw_model_picker = true;
            if result.state.is_failure() {
                return Err(format!(
                    "Model picker patch failed for {}: {}",
                    file.path,
                    result.state.as_str()
                ));
            }
        }
        if is_power_picker && result.state.is_failure() {
            return Err(format!(
                "Reasoning effort patch failed for {}: {}",
                file.path,
                result.state.as_str()
            ));
        }
        if is_power_selection_scope && result.state.is_failure() {
            return Err(format!(
                "Reasoning effort scope patch failed for {}: {}",
                file.path,
                result.state.as_str()
            ));
        }
        if (is_power_slider_js || is_power_slider_css) && result.state.is_failure() {
            return Err(format!(
                "Custom model picker UI patch failed for {}: {}",
                file.path,
                result.state.as_str()
            ));
        }
        if is_composer_collapsed_reasoning_label
            && (result.state.is_failure() || result.patch_name != Some("composer-reasoning-labels"))
        {
            return Err(format!(
                "Composer reasoning label patch failed for {}: {}",
                file.path,
                result.state.as_str()
            ));
        }
        if is_reasoning_label_defaults
            && (result.state.is_failure()
                || result.patch_name != Some("reasoning-effort-label-defaults"))
        {
            return Err(format!(
                "Reasoning label defaults patch failed for {}: {}",
                file.path,
                result.state.as_str()
            ));
        }

        if result.state == PatchDisposition::Patched {
            if let Some(patch_name) = result.patch_name {
                if !verify_entry_integrity(&file.entry, &bytes) {
                    return Err(format!(
                        "Patched ASAR integrity mismatch for {}.",
                        file.path
                    ));
                }
                append_unique(&mut already_patch_names, patch_name);
                for additional in result.additional_patch_names {
                    append_unique(&mut already_patch_names, additional);
                }
            }
        }
        if result.state == PatchDisposition::Patchable {
            let patch_name = result
                .patch_name
                .ok_or_else(|| format!("缺少 patch 名称: {}", file.path))?;
            let replacement = result
                .bytes
                .ok_or_else(|| format!("缺少 patch 内容: {}", file.path))?;
            targets.push(PatchTarget {
                path: file.path,
                entry_path: file.entry_path,
                entry: file.entry,
                offset,
                bytes: replacement,
            });
            append_unique(&mut patch_names, patch_name);
            for additional in result.additional_patch_names {
                append_unique(&mut patch_names, additional);
            }
        }
    }

    if !saw_model_picker {
        return Err("Did not find Codex model picker gate in app.asar.".to_string());
    }
    if !patch_names.iter().any(|name| name == "model-picker")
        && !already_patch_names
            .iter()
            .any(|name| name == "model-picker")
    {
        return Err("Did not patch or verify the Codex model picker gate in app.asar.".to_string());
    }
    if saw_power_picker && !saw_power_selection_scope {
        return Err(
            "Did not find the model picker Power slider scope call in app.asar.".to_string(),
        );
    }
    if !saw_power_slider_js || !saw_power_slider_css {
        return Err(
            "Did not find both model picker Power slider JS and CSS assets in app.asar."
                .to_string(),
        );
    }
    if !saw_composer_collapsed_reasoning_label || !saw_reasoning_label_defaults {
        return Err(
            "Did not find both composer and default reasoning label assets in app.asar."
                .to_string(),
        );
    }
    if !saw_stable_metadata_retry
        || !patch_names
            .iter()
            .chain(already_patch_names.iter())
            .any(|name| name == "stable-metadata-git-retry")
    {
        return Err(
            "Did not patch or verify the stable metadata Git retry guard in app.asar.".to_string(),
        );
    }

    let all_patch_names = combined_patch_names(&already_patch_names, &patch_names);
    if targets.is_empty() {
        let patched_asar_hash = sha256_hex(&fs::read(&request.app_asar_path).map_err(|error| {
            format!(
                "读取已 patch app.asar 失败 {}: {error}",
                request.app_asar_path.display()
            )
        })?);
        let state = patch_state_value(
            request,
            "already-patched",
            &patch_version,
            &source_asar_hash,
            source_codex_version.as_deref(),
            &patched_asar_hash,
            &all_patch_names,
            None,
            None,
            None,
        )?;
        write_patch_state(&request.patch_state_path, &state)?;
        return Ok(state);
    }

    let backup_path = create_backup(&request.app_asar_path, &request.backup_dir)?;
    let previous_patch_state = match fs::read(&request.patch_state_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "读取旧 patch 状态失败 {}: {error}",
                request.patch_state_path.display()
            ));
        }
    };
    let write_result = (|| -> Result<Value, String> {
        let write_result = write_patched_asar(&request.app_asar_path, &mut asar, &mut targets)?;
        let verification_asar = read_asar_header(&request.app_asar_path)?;
        let verification_files = list_asar_files(&verification_asar.header)?;
        for target in &targets {
            let file = verification_files
                .iter()
                .find(|file| file.path == target.path)
                .ok_or_else(|| {
                    format!(
                        "Patch verification failed for {}: missing patched file",
                        target.path
                    )
                })?;
            let (_, bytes) =
                read_asar_entry(&request.app_asar_path, verification_asar.files_offset, file)?;
            if !verify_entry_patch(&file.path, &bytes)? {
                return Err(format!("Patch verification failed for {}", target.path));
            }
            if !verify_entry_integrity(&file.entry, &bytes) {
                return Err(format!(
                    "Patch integrity verification failed for {}",
                    target.path
                ));
            }
        }
        let patched_asar_hash = sha256_hex(&fs::read(&request.app_asar_path).map_err(|error| {
            format!(
                "读取 patch 后 app.asar 失败 {}: {error}",
                request.app_asar_path.display()
            )
        })?);
        let state = patch_state_value(
            request,
            "patched",
            &patch_version,
            &source_asar_hash,
            source_codex_version.as_deref(),
            &patched_asar_hash,
            &all_patch_names,
            Some(&backup_path),
            Some(&write_result.patched_files),
            Some(&write_result.header_hash),
        )?;
        write_patch_state(&request.patch_state_path, &state)?;
        Ok(state)
    })();

    match write_result {
        Ok(state) => Ok(state),
        Err(error) => {
            let mut restore_errors = Vec::new();
            if let Err(restore_error) = fs::copy(&backup_path, &request.app_asar_path) {
                restore_errors.push(format!(
                    "恢复 app.asar 失败 {} -> {}: {restore_error}",
                    backup_path.display(),
                    request.app_asar_path.display()
                ));
            }
            if let Err(restore_error) =
                restore_patch_state(&request.patch_state_path, previous_patch_state.as_deref())
            {
                restore_errors.push(restore_error);
            }
            if restore_errors.is_empty() {
                Err(format!(
                    "Patch failed; restored the controlled app.asar backup: {error}"
                ))
            } else {
                Err(format!(
                    "Patch failed and backup restore failed: {error}; {}",
                    restore_errors.join("; ")
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_test_dir(name: &str) -> PathBuf {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "codexdeck-model-picker-{name}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create test directory");
        path
    }

    fn write_asar_fixture(path: &Path, header: &Value, contents: &[u8]) -> Vec<u8> {
        let header_bytes = serde_json::to_vec(header).expect("serialize ASAR header");
        write_asar_header_bytes_fixture(path, &header_bytes, contents);
        header_bytes
    }

    fn write_asar_header_bytes_fixture(path: &Path, header_bytes: &[u8], contents: &[u8]) {
        let mut asar = vec![0_u8; 16];
        let string_pickle_size = (4 + header_bytes.len() + 3) & !3;
        let header_pickle_size = 4 + string_pickle_size;
        asar[0..4].copy_from_slice(&4_u32.to_le_bytes());
        asar[4..8].copy_from_slice(&(header_pickle_size as u32).to_le_bytes());
        asar[8..12].copy_from_slice(&(string_pickle_size as u32).to_le_bytes());
        asar[12..16].copy_from_slice(&(header_bytes.len() as u32).to_le_bytes());
        asar.extend_from_slice(&header_bytes);
        asar.resize(8 + header_pickle_size, 0);
        asar.extend_from_slice(contents);
        fs::write(path, &asar).expect("write ASAR fixture");
    }

    #[test]
    fn asar_header_roundtrip_preserves_entry_layout() {
        let directory = temporary_test_dir("asar-roundtrip");
        let asar_path = directory.join("app.asar");
        let header = serde_json::json!({
            "files": {
                "webview": {
                    "files": {
                        "assets": {
                            "files": {
                                "entry.js": { "offset": "0", "size": 3 }
                            }
                        }
                    }
                }
            }
        });
        let header_bytes = write_asar_fixture(&asar_path, &header, b"abc");

        let asar = read_asar_header(&asar_path).expect("read ASAR header");
        assert_eq!(asar.header_bytes, header_bytes);
        assert_eq!(
            serialize_asar_header(&asar).expect("roundtrip header"),
            header_bytes
        );
        let files = list_asar_files(&asar.header).expect("list ASAR files");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "webview/assets/entry.js");
        let (offset, bytes) =
            read_asar_entry(&asar_path, asar.files_offset, &files[0]).expect("read ASAR entry");
        assert_eq!(offset, asar.files_offset);
        assert_eq!(bytes, b"abc");

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn asar_header_padding_is_excluded_from_file_entries() {
        let directory = temporary_test_dir("asar-header-padding");
        let asar_path = directory.join("app.asar");
        let mut header_bytes = br#"{"files":{"entry.js":{"offset":"0","size":3}}}"#.to_vec();
        while header_bytes.len() % 4 != 3 {
            header_bytes.push(b' ');
        }
        write_asar_header_bytes_fixture(&asar_path, &header_bytes, b"abc");

        let asar = read_asar_header(&asar_path).expect("read padded ASAR header");
        assert_eq!(asar.files_offset, 16 + header_bytes.len() as u64 + 1);
        let entry = list_asar_files(&asar.header)
            .expect("list padded fixture files")
            .into_iter()
            .next()
            .expect("padded fixture entry");
        let (_, bytes) =
            read_asar_entry(&asar_path, asar.files_offset, &entry).expect("read padded entry");
        assert_eq!(bytes, b"abc");

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn asar_header_manual_non_dictionary_order_roundtrips() {
        let directory = temporary_test_dir("asar-header-order");
        let asar_path = directory.join("app.asar");
        let header_bytes = br#"{"z":0,"files":{"entry.js":{"size":3,"offset":"0"}},"a":1}"#;
        write_asar_header_bytes_fixture(&asar_path, header_bytes, b"abc");

        let asar = read_asar_header(&asar_path).expect("parse hand-written header");
        assert_eq!(asar.header_bytes, header_bytes);
        assert_eq!(
            serialize_asar_header(&asar).expect("serialize hand-written header"),
            header_bytes
        );

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn in_place_asar_write_updates_header_integrity_without_resizing_it() {
        let directory = temporary_test_dir("asar-integrity-write");
        let asar_path = directory.join("app.asar");
        let zero_hash = "0".repeat(64);
        let header = serde_json::json!({
            "files": {
                "entry.js": {
                    "offset": "0",
                    "size": 3,
                    "integrity": {
                        "algorithm": "SHA256",
                        "blockSize": 4,
                        "hash": zero_hash,
                        "blocks": ["0000000000000000000000000000000000000000000000000000000000000000"]
                    }
                }
            }
        });
        let original_header = write_asar_fixture(&asar_path, &header, b"old");
        let mut asar = read_asar_header(&asar_path).expect("read ASAR fixture");
        let file = list_asar_files(&asar.header)
            .expect("list fixture files")
            .into_iter()
            .next()
            .expect("fixture entry");
        let mut targets = vec![PatchTarget {
            path: file.path.clone(),
            entry_path: file.entry_path.clone(),
            entry: file.entry.clone(),
            offset: asar.files_offset + file.offset,
            bytes: b"new".to_vec(),
        }];
        write_patched_asar(&asar_path, &mut asar, &mut targets).expect("write patched ASAR");

        let verified = read_asar_header(&asar_path).expect("read patched ASAR");
        assert_eq!(verified.header_bytes.len(), original_header.len());
        let entry = list_asar_files(&verified.header)
            .expect("list patched files")
            .into_iter()
            .next()
            .expect("patched entry");
        let (_, bytes) =
            read_asar_entry(&asar_path, verified.files_offset, &entry).expect("read patched entry");
        assert_eq!(bytes, b"new");
        assert!(verify_entry_integrity(&entry.entry, &bytes));

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn atomic_state_writer_replaces_and_restores_previous_bytes() {
        let directory = temporary_test_dir("atomic-state");
        let state_path = directory.join("model-picker-patch-state.json");
        fs::write(&state_path, b"old state\n").expect("write old state");

        write_bytes_atomically(&state_path, b"new state\n").expect("replace state atomically");
        assert_eq!(
            fs::read(&state_path).expect("read replacement"),
            b"new state\n"
        );
        restore_patch_state(&state_path, Some(b"old state\n")).expect("restore previous state");
        assert_eq!(
            fs::read(&state_path).expect("read restored state"),
            b"old state\n"
        );
        restore_patch_state(&state_path, None).expect("remove restored state");
        assert!(!state_path.exists());

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn model_gate_patch_is_byte_length_preserving() {
        let source = ",s=i&&e!==`amazonBedrock`;";
        let patch = replace_model_picker_gate(source);
        assert_eq!(patch.state, PatchDisposition::Patchable);
        assert_eq!(
            patch.text.expect("replacement").as_bytes().len(),
            source.as_bytes().len()
        );
    }

    #[test]
    fn stable_metadata_non_git_guard_is_length_preserving_and_keeps_retry_policy() {
        let source = format!(
            "function aV(e){{{STABLE_METADATA_NON_RETRY_BEFORE}}}var oV=e((()=>{{}}));\
             function sV(e,t){{return{{retry:(e,t)=>(t instanceof Error?t.message:String(t)).includes(`Unknown method: process/spawn`)||aV(t)?!1:e<uV,retryDelay:()=>lV,queryFn:()=>`stable-metadata`,modelProviders:null,previous:{{modelProviders:null}}}}}}\
             var lV,uV,dV=e((()=>{{lV=1e3,uV=3}}));"
        );

        let first = replace_stable_metadata_non_retry_guard(&source);
        assert_eq!(first.state, PatchDisposition::Patchable);
        let patched = first.text.expect("patched stable metadata guard");
        assert_eq!(patched.len(), source.len());
        assert!(patched.contains(STABLE_METADATA_NON_RETRY_AFTER));
        assert!(patched.contains("Unknown method: process/spawn"));
        assert!(patched.contains("lV=1e3,uV=3"));
        assert!(!patched.contains("uV=0"));
        assert!(!patched.contains(HISTORY_PROVIDER_FILTER_BEFORE));
        assert_eq!(
            count_occurrences(&patched, HISTORY_PROVIDER_FILTER_AFTER),
            2
        );

        assert_eq!(
            replace_stable_metadata_non_retry_guard(&patched).state,
            PatchDisposition::Patched
        );
    }

    #[test]
    fn unknown_and_ambiguous_model_gates_are_rejected() {
        assert_eq!(
            replace_model_picker_gate("unrelated source").state,
            PatchDisposition::Missing
        );
        assert_eq!(
            replace_model_picker_gate(",s=i&&e!==`amazonBedrock`;,u=s&&e!==`amazonBedrock`,").state,
            PatchDisposition::Ambiguous
        );
    }

    #[test]
    fn entry_integrity_updates_and_detects_changes() {
        let mut entry = serde_json::json!({
            "offset": "0",
            "size": 5,
            "integrity": {
                "algorithm": "SHA256",
                "blockSize": 2,
                "hash": "",
                "blocks": []
            }
        });
        let bytes = b"abcde";
        update_entry_integrity(&mut entry, bytes).expect("update integrity");
        assert!(verify_entry_integrity(&entry, bytes));
        assert!(!verify_entry_integrity(&entry, b"abcdf"));
        assert_eq!(
            entry["integrity"]["blocks"]
                .as_array()
                .expect("blocks")
                .len(),
            3
        );
    }

    #[test]
    fn integrity_arrays_are_rejected_for_update_and_verify() {
        let mut entry = serde_json::json!({ "integrity": [] });
        assert!(update_entry_integrity(&mut entry, b"abc").is_err());
        assert!(!verify_entry_integrity(&entry, b"abc"));
    }

    #[test]
    fn block_size_matches_js_number_and_nullish_semantics() {
        fn with_block_size(value: Value) -> Map<String, Value> {
            let mut integrity = Map::new();
            integrity.insert("blockSize".to_string(), value);
            integrity
        }

        for value in [
            Value::from(0),
            Value::Bool(false),
            Value::String(String::new()),
        ] {
            let integrity = with_block_size(value);
            assert_eq!(
                integrity_block_size_for_update(&integrity).expect("update default"),
                INTEGRITY_DEFAULT_BLOCK_SIZE
            );
            assert!(integrity_block_size_for_verify(&integrity).is_err());
        }

        for (value, expected) in [
            (Value::String("4e0".to_string()), 4_usize),
            (Value::String("0x10".to_string()), 16_usize),
        ] {
            let integrity = with_block_size(value);
            assert_eq!(
                integrity_block_size_for_update(&integrity).expect("update Number coercion"),
                expected
            );
            assert_eq!(
                integrity_block_size_for_verify(&integrity).expect("verify Number coercion"),
                expected
            );
        }
    }

    #[test]
    fn power_selection_closing_matches_the_js_regex() {
        assert!(has_invalid_power_selection_closing("x]]}));"));
        assert!(has_invalid_power_selection_closing("x] \n\t ]}));"));
        assert!(!has_invalid_power_selection_closing("x]]})));"));
        assert!(!has_invalid_power_selection_closing("x] ]});"));
    }

    #[test]
    fn invalid_utf8_entries_use_node_style_lossy_decoding() {
        let bytes = [0xff_u8];
        assert_eq!(
            patch_entry_bytes("webview/assets/unrelated.js", &bytes)
                .expect("lossy patch dispatch")
                .state,
            PatchDisposition::Missing
        );
        assert!(verify_entry_patch("webview/assets/unrelated.js", &bytes)
            .expect("lossy verification dispatch"));
    }

    #[test]
    fn embedded_picker_payloads_match_their_declared_hashes() {
        assert!(embedded_custom_picker_component()
            .expect("component payload")
            .contains("__COMPONENT__"));
        assert!(embedded_custom_picker_css()
            .expect("CSS payload")
            .contains(CUSTOM_PICKER_CSS_MARKER));
    }

    #[test]
    fn embedded_picker_uses_compact_geometry_and_complete_ultra_effects() {
        let component = embedded_custom_picker_component().expect("component payload");
        let css = embedded_custom_picker_css().expect("CSS payload");

        assert!(component.contains("codexdeck-model-picker-ui-v7"));
        assert!(component.contains("cdp-track-particles"));
        assert!(component.contains("cdp-energy-arrows"));
        assert!(css.contains(".cdpv8"));
        assert!(!css.contains("cursor:crosshair"));
        assert!(css.contains("--panel-width:344px"));
        assert!(css.contains("height:128px"));
        assert!(css.contains(
            ".cdp-ultra{display:flex;grid-column:3;grid-row:1 / 4;width:100px;height:309px"
        ));
        assert!(css.contains("transform:scale(.4);transform-origin:top center"));
        assert!(css.contains(
            ".cdp-ultra>button{--knob-top: 28px;--stem-top: 48px;--stem-energy-band: 44px;position:relative;display:block;overflow:hidden;width:100px;height:260px"
        ));
        assert!(css.contains("top:24px;bottom:22px;left:28px;width:44px"));
        assert!(css.contains("left:44px;width:12px;height:165px;overflow:hidden"));
        assert!(css.contains("left:19px;width:62px;height:62px"));
        assert!(css.contains("animation:cdp-energy-arrow 1.35s"));
        assert!(component.contains("Array.from({length:16}"));
        assert!(component.contains("Array.from({length:3}"));
        assert!(css.contains("@keyframes cdp-track-particle"));
        assert!(css.contains("@keyframes cdp-energy-arrow"));
        assert!(css.contains("@keyframes cdp-ultra-particle"));
    }

    #[test]
    fn model_list_filter_compacts_and_pads_the_max_reasoning_patch() {
        let source = concat!(
            "availableModels useHiddenModels enabledReasoningEfforts",
            ",s=i&&e!==`amazonBedrock`,t=u.some(()=>0),",
            "({reasoningEffort:r})=>f(r)&&g.has(r)"
        );
        let patch = replace_model_list_filter(source);
        assert_eq!(patch.state, PatchDisposition::Patchable);
        let patched = patch.text.expect("patched model-list filter");
        assert_eq!(patched.as_bytes().len(), source.as_bytes().len());
        assert!(patched.contains(",s=0,t=u.some("));
        assert!(patched.contains("r===`max`||g.has(r)"));
    }

    #[test]
    fn power_options_filter_and_menu_width_patch_together() {
        let collapsed_label = concat!(
            "function Ue(e,t){return`${e.modelLabel} ${t.formatMessage(Ke[e.reasoningEffort])}`}",
            "var We,Ge,Z,Ke,qe=e((()=>{Ke={...L,medium:s({id:`composer.modelPicker.work.reasoning.standard.label`,defaultMessage:`Standard`}),",
            "high:s({id:`composer.modelPicker.work.reasoning.extended.label`,defaultMessage:`Extended`})}}))"
        );
        let visible_menu_labels = concat!(
            "let Pe=L[q],Fe;t[32]===Pe?Fe=t[33]:(Fe=(0,$.jsx)(u,{...Pe}),t[32]=Pe,t[33]=Fe);",
            "children:(0,$.jsx)(u,{...L[t]});children:(0,$.jsx)(u,{...L[t]})"
        );
        let source = format!(
            "q=[{LEGACY_SOL_POWER_SELECTIONS}];{LEGACY_POWER_SELECTION_FILTER};x(`w-56`,y);{collapsed_label};{visible_menu_labels}"
        );
        let patch = replace_power_selections(&source);
        assert_eq!(patch.state, PatchDisposition::Patchable);
        let patched = patch.text.expect("patched Power options");
        assert_eq!(patched.as_bytes().len(), source.as_bytes().len());
        assert!(has_complete_power_selections(&patched));
        assert!(patched.contains("t.modelLabel=e?.find(e=>e.model==t.model)?.displayName"));
        assert!(patched.contains(
            r#"{"low":"Low","medium":"Medium","high":"High","xhigh":"Extra high","max":"Max","ultra":"ULTRA"}"#
        ));
        assert!(!patched.contains("formatMessage(Ke[e.reasoningEffort])"));
        assert!(patched.contains("Fe=Pe.defaultMessage"));
        assert_eq!(
            count_occurrences(&patched, "children:L[t].defaultMessage"),
            2
        );
        assert!(!patched.contains("children:(0,$.jsx)(u,{...L[t]})"));
        assert_eq!(
            replace_power_selections(&patched).state,
            PatchDisposition::Patched
        );
    }

    #[test]
    fn split_ultra_power_options_patch_and_menu_width_together() {
        let collapsed_label = concat!(
            "function Qe(e,t){return`${e.modelLabel} ${t.formatMessage(Ke[e.reasoningEffort])}`};",
            "var We,Ge,Z,Ke,qe=e((()=>{Ke={...L,medium:s({id:`composer.modelPicker.work.reasoning.standard.label`,defaultMessage:`Standard`}),",
            "high:s({id:`composer.modelPicker.work.reasoning.extended.label`,defaultMessage:`Extended`})}}))"
        );
        let visible_menu_labels = concat!(
            "let Pe=L[q],Fe;t[32]===Pe?Fe=t[33]:(Fe=(0,$.jsx)(u,{...Pe}),t[32]=Pe,t[33]=Fe);",
            "children:(0,$.jsx)(u,{...L[t]});children:(0,$.jsx)(u,{...L[t]})"
        );
        let source = format!(
            concat!(
                "q=[",
                "{{id:`gpt-5.6-terra:low`,model:`gpt-5.6-terra`,modelLabel:`5.6 Terra`,reasoningEffort:`low`}},",
                "{{id:`gpt-5.6-sol:low`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`low`}},",
                "{{id:`gpt-5.6-sol:medium`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`medium`}},",
                "{{id:`gpt-5.6-sol:high`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`high`}},",
                "{{id:`gpt-5.6-sol:xhigh`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`xhigh`}}",
                "];",
                "le={{id:`gpt-5.6-sol:ultra`,model:`gpt-5.6-sol`,modelLabel:`5.6 Sol`,reasoningEffort:`ultra`}};",
                "ue=[];",
                "function ae(e,t=!1){{let n=K(t?[...q,le]:q,e);if(n.length>=4)return n;let r=K(ue,e);return r.length>=4?r:[]}};",
                "x(`w-56`,y);{collapsed_label};{visible_menu_labels}"
            ),
            collapsed_label = collapsed_label,
            visible_menu_labels = visible_menu_labels,
        );

        assert!(contains_power_picker_signature(&source));
        let patch = replace_power_selections(&source);
        assert_eq!(patch.state, PatchDisposition::Patchable);
        let patched = patch.text.expect("patched split Ultra Power options");
        assert_eq!(patched.as_bytes().len(), source.as_bytes().len());
        assert!(has_complete_power_selections(&patched));
        assert!(patched.contains("`cdpw`"));
        assert!(patched.contains("terra,sol,luna"));
    }

    #[test]
    fn scope_history_and_zh_label_patches_are_length_preserving() {
        let scope_source = concat!(
            "import{h as z}from\"./model-and-reasoning-dropdown-a.js\";",
            "r=x(a,n),p=z(a);\n//# sourceMappingURL=fixture\npowerSelections:"
        );
        let scope_patch = replace_power_selection_scope_call(scope_source);
        assert_eq!(scope_patch.state, PatchDisposition::Patchable);
        let scoped = scope_patch.text.expect("scoped Power selection");
        assert_eq!(scoped.as_bytes().len(), scope_source.as_bytes().len());
        assert!(scoped.contains("p=z(a,n)//# sourceMappingURL="));

        let history = patch_entry_bytes("webview/assets/history.js", b"modelProviders:null")
            .expect("patch provider history");
        assert_eq!(history.state, PatchDisposition::Patchable);
        assert_eq!(
            history.bytes.expect("history bytes"),
            b"modelProviders:[]  "
        );

        let zh_source = concat!(
            "\"composer.mode.local.reasoning.max.label\":`最高`,",
            "\"composer.mode.local.reasoning.ultra.label\":`极高`"
        );
        let zh = patch_entry_bytes("webview/assets/zh-CN-fixture.js", zh_source.as_bytes())
            .expect("patch Chinese reasoning labels");
        assert_eq!(zh.state, PatchDisposition::Patchable);
        let zh_bytes = zh.bytes.expect("Chinese patch bytes");
        assert_eq!(zh_bytes.len(), zh_source.as_bytes().len());
        let zh_text = std::str::from_utf8(&zh_bytes).expect("Chinese patch UTF-8");
        assert!(zh_text.contains("reasoning.max.label\":`Max`"));
        assert!(zh_text.contains("reasoning.ultra.label\":`超高`"));
    }

    #[test]
    fn composer_collapsed_reasoning_labels_use_english_defaults() {
        let composer_source = concat!(
            "expandedContentWidth,labelCandidates,WorkTriggerEffortLabel;",
            "w=e=>({...e,effortLabel:C.formatMessage(Fv[e.reasoningEffort])});",
            "let te=p===`ultra`,ne=Fv[p],q;",
            "t[26]===ne?q=t[27]:(q=(0,zO.jsx)(Z,{...ne}),t[26]=ne,t[27]=q);"
        );
        let composer = patch_entry_bytes(
            "webview/assets/composer-fixture.js",
            composer_source.as_bytes(),
        )
        .expect("patch composer collapsed label");
        assert_eq!(composer.state, PatchDisposition::Patchable);
        let composer_bytes = composer.bytes.expect("patched composer bytes");
        assert_eq!(composer_bytes.len(), composer_source.len());
        let composer_text = std::str::from_utf8(&composer_bytes).expect("composer UTF-8");
        assert!(composer_text.contains("Fv[e.reasoningEffort].defaultMessage"));
        assert!(composer_text.contains("q=ne.defaultMessage"));
        assert!(!composer_text.contains("q=(0,zO.jsx)(Z,{...ne})"));
        assert_eq!(
            patch_entry_bytes("webview/assets/composer-fixture.js", &composer_bytes)
                .expect("verify composer collapsed label")
                .state,
            PatchDisposition::Patched
        );

        let defaults_source = concat!(
            "defaultMessage:`Light`;",
            "defaultMessage:`Extra High`;",
            "defaultMessage:`Ultra`"
        );
        let defaults = patch_entry_bytes(
            "webview/assets/reasoning-effort-label-messages-fixture.js",
            defaults_source.as_bytes(),
        )
        .expect("patch reasoning defaults");
        assert_eq!(defaults.state, PatchDisposition::Patchable);
        let defaults_bytes = defaults.bytes.expect("patched reasoning defaults");
        assert_eq!(defaults_bytes.len(), defaults_source.len());
        let defaults_text = std::str::from_utf8(&defaults_bytes).expect("defaults UTF-8");
        assert!(defaults_text.contains("defaultMessage:`Low`"));
        assert!(defaults_text.contains("defaultMessage:`Extra high`"));
        assert!(defaults_text.contains("defaultMessage:`ULTRA`"));
        assert_eq!(
            patch_entry_bytes(
                "webview/assets/reasoning-effort-label-messages-fixture.js",
                &defaults_bytes,
            )
            .expect("verify reasoning defaults")
            .state,
            PatchDisposition::Patched
        );
    }

    #[test]
    fn scope_lookup_uses_utf16_code_unit_windows() {
        let unicode_padding = "中".repeat(2_000);
        let source = format!(
            concat!(
                "import{{h as z}}from\"./model-and-reasoning-dropdown-a.js\";",
                "n=s.model;{}p=z(a);\n//# sourceMappingURL=fixture\npowerSelections:"
            ),
            unicode_padding,
        );
        let patch = replace_power_selection_scope_call(&source);
        assert_eq!(patch.state, PatchDisposition::Patchable);
        let patched = patch.text.expect("UTF-16 scope patch");
        assert_eq!(patched.as_bytes().len(), source.as_bytes().len());
        assert!(patched.contains("p=z(a,n)//# sourceMappingURL="));
    }

    #[test]
    fn patch_timestamp_uses_js_iso_milliseconds_in_utc() {
        let value = time::Date::from_calendar_date(2026, time::Month::July, 13)
            .expect("date")
            .with_hms_milli(8, 9, 10, 11)
            .expect("time")
            .assume_utc();
        assert_eq!(format_js_iso_timestamp(value), "2026-07-13T08:09:10.011Z");
    }
}
