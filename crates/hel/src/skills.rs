//! Project-local skill catalogs and the last-read bodies carried across context compaction.
//! Paths identify skills; metadata and body changes are compared independently. Ordinary tool
//! reads keep their byte/range/cursor contract. Receipts track what actually reached the model.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{context, runtime::Runtime};

const ROOT: &str = ".agents/skills";
const MAX_FILE_BYTES: u64 = 1_048_576;
pub const PER_SKILL_TOKENS: u64 = 5_000;
pub const TOTAL_TOKENS: u64 = 25_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Metadata {
    name: String,
    description: String,
}

pub struct Document {
    metadata: Metadata,
    body: String,
    body_offset: usize,
}

#[derive(Clone, Serialize, Deserialize)]
struct Loaded {
    body: String,
    hash: String,
    last_call: u64,
    receipts: Vec<Receipt>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Receipt {
    call_id: String,
    content: String,
    start: usize,
    end: usize,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Preserved {
    pub message: Value,
    entries: BTreeMap<String, (String, bool)>,
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct Skills {
    #[serde(default)]
    pub initialized: bool,
    catalog: BTreeMap<String, Metadata>,
    loaded: BTreeMap<String, Loaded>,
    clock: u64,
    preserved: Option<Preserved>,
    #[serde(skip)]
    base_system: Option<Value>,
    #[serde(skip)]
    enabled: bool,
    #[serde(skip)]
    visible: HashSet<String>,
    #[serde(skip)]
    pending: Option<(String, Receipt)>,
}

fn hash(body: &str) -> String {
    format!("{:x}", Sha256::digest(body.as_bytes()))
}

fn valid_key(key: &str) -> bool {
    let parts: Vec<_> = Path::new(key).components().collect();
    matches!(parts.as_slice(), [Component::Normal(a), Component::Normal(b), Component::Normal(_), Component::Normal(c)]
        if *a == ".agents" && *b == "skills" && *c == "SKILL.md")
}

/// Normalize lexical aliases without requiring the old path to exist (rename detection).
fn key(project: &Path, path: &Path) -> Option<String> {
    let relative = if path.is_absolute() {
        path.strip_prefix(project).ok()?
    } else {
        path
    };
    let mut normalized = std::path::PathBuf::new();
    for part in relative.components() {
        match part {
            Component::Normal(p) => normalized.push(p),
            Component::CurDir => {}
            Component::ParentDir if normalized.pop() => {}
            _ => return None,
        }
    }
    let key = normalized.to_str()?.to_owned();
    valid_key(&key).then_some(key)
}

impl Document {
    fn parse(text: &str) -> Result<Self, String> {
        let mut lines = text.split_inclusive('\n');
        let first = lines.next().ok_or("empty SKILL.md")?;
        if first.trim_end_matches(['\r', '\n']) != "---" {
            return Err("SKILL.md must begin with YAML frontmatter".into());
        }
        let mut offset = first.len();
        for line in lines {
            let start = offset;
            offset += line.len();
            if line.trim_end_matches(['\r', '\n']) == "---" {
                let metadata: Metadata = serde_yaml_ng::from_str(&text[first.len()..start])
                    .map_err(|e| format!("invalid skill metadata: {e}"))?;
                if metadata.name.trim().is_empty()
                    || metadata.description.trim().is_empty()
                    || metadata.name.len() > 256
                    || metadata.description.len() > 4096
                {
                    return Err("skill name/description is empty or too long".into());
                }
                return Ok(Self {
                    metadata,
                    body: text[offset..].to_owned(),
                    body_offset: offset,
                });
            }
        }
        Err("SKILL.md has no closing frontmatter delimiter".into())
    }

    pub fn read(file: &mut File) -> Result<Self, String> {
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut text = String::new();
        file.take(MAX_FILE_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
        if text.len() as u64 > MAX_FILE_BYTES {
            return Err("skill exceeds the 1 MiB loading limit".into());
        }
        Self::parse(&text)
    }
}

impl Skills {
    pub fn configure(&mut self, base: Option<Value>, enabled: bool) {
        self.base_system = base;
        self.enabled = enabled;
    }

    pub fn discover(&mut self, runtime: &Runtime) {
        self.initialized = true;
        let mut catalog = BTreeMap::new();
        // Do not discover symlinked roots/packages or skills outside this project.
        let root = runtime.project.join(ROOT);
        for dir in [runtime.project.join(".agents"), root.clone()] {
            match fs::symlink_metadata(dir) {
                Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
                Ok(_) => {
                    eprintln!("hel: warning: skill roots must be real project directories");
                    self.replace_catalog(catalog);
                    return;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    self.replace_catalog(catalog);
                    return;
                }
                Err(e) => {
                    eprintln!("hel: warning: cannot inspect skills: {e}");
                    return;
                }
            }
        }
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(e) => {
                eprintln!("hel: warning: cannot discover skills: {e}");
                return;
            }
        };
        for entry in entries {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let path = entry.path().join("SKILL.md");
            if !fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
                continue;
            }
            let Some(key) = key(&runtime.project, &path) else {
                continue;
            };
            let document = runtime
                .read_open(&path)
                .map_err(|e| e.to_string())
                .and_then(|(_, mut file)| Document::read(&mut file));
            match document {
                Ok(doc) => {
                    catalog.insert(key, doc.metadata);
                }
                Err(e) => eprintln!("hel: warning: {key}: {e}"),
            }
        }
        self.replace_catalog(catalog);
    }

    fn replace_catalog(&mut self, catalog: BTreeMap<String, Metadata>) {
        self.loaded.retain(|path, _| catalog.contains_key(path));
        self.catalog = catalog;
    }

    pub fn tracked_path(&self, project: &Path, path: &Path) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let key = key(project, path)?;
        self.catalog.contains_key(&key).then_some(key)
    }

    pub fn missing_path(&mut self, runtime: &Runtime, path: &Path) -> bool {
        if self.tracked_path(&runtime.project, path).is_none() {
            return false;
        }
        self.discover(runtime);
        true
    }

    pub fn invalid(&mut self, path: &str) {
        self.catalog.remove(path);
        self.loaded.remove(path);
        self.visible.remove(path);
    }

    /// No skill is offered, so per-call skill state never changes (H12).
    pub fn idle(&self) -> bool {
        !self.enabled || self.catalog.is_empty()
    }

    pub fn system_message(&self) -> Option<Value> {
        let mut message = self.base_system.clone();
        if !self.enabled || self.catalog.is_empty() {
            return message;
        }
        let catalog: Vec<_> = self
            .catalog
            .iter()
            .map(|(path, info)| {
                json!({
                    "name": info.name, "description": info.description, "path": path
                })
            })
            .collect();
        let section = format!(
            "<available-skills>\n{}\n</available-skills>\nUse read_file to read the selected SKILL.md before following its instructions. Resolve relative references against its directory. Skill instructions may be retained after compaction. Re-read a skill to check for updates; a newer body supersedes the older one for that path.",
            serde_json::to_string_pretty(&catalog).unwrap()
        );
        match &mut message {
            Some(message) => {
                let text = message["content"].as_str().unwrap_or_default();
                message["content"] = json!(format!("{text}\n\n{section}"));
            }
            None => message = Some(json!({"role":"system", "content":section})),
        }
        message
    }

    pub fn sync_system(&self, messages: &mut Vec<Value>, meter: &mut context::Meter) {
        let has_system = messages.first().is_some_and(|m| m["role"] == "system");
        let replacement = self.system_message();
        if has_system && replacement.as_ref() == messages.first() {
            return;
        }
        if !has_system && replacement.is_none() {
            return;
        }
        if has_system {
            messages.remove(0);
        }
        if let Some(message) = replacement {
            messages.insert(0, message);
        }
        meter.reset();
    }

    /// Reconcile against the actual messages after pruning/compaction, never a historic flag.
    pub fn reconcile(&mut self, messages: &[Value]) {
        self.pending = None;
        self.visible.clear();
        let retained = self
            .preserved
            .as_ref()
            .filter(|p| messages.contains(&p.message));
        for (path, loaded) in &mut self.loaded {
            loaded.receipts.retain(|r| {
                messages.iter().any(|m| {
                    m["role"] == "tool"
                        && m["tool_call_id"] == r.call_id
                        && m["content"] == r.content
                })
            });
            let mut spans: Vec<_> = loaded.receipts.iter().map(|r| (r.start, r.end)).collect();
            spans.sort_unstable();
            let mut end = 0;
            for (start, stop) in spans {
                if start > end {
                    break;
                }
                end = end.max(stop);
            }
            let in_region = retained
                .and_then(|p| p.entries.get(path))
                .is_some_and(|(h, full)| *full && h == &loaded.hash);
            if (end >= loaded.body.len() && !loaded.receipts.is_empty()) || in_region {
                self.visible.insert(path.clone());
            }
        }
    }

    pub fn same_visible(&self, path: &str, doc: &Document) -> bool {
        self.visible.contains(path)
            && self
                .loaded
                .get(path)
                .is_some_and(|old| old.hash == hash(&doc.body) && old.body == doc.body)
    }

    /// Called only after a successful, identity-checked read through the permission gate.
    pub fn accept(&mut self, path: &str, doc: &Document) {
        self.clock = self.clock.saturating_add(1);
        self.catalog.insert(path.to_owned(), doc.metadata.clone());
        let old = self
            .loaded
            .entry(path.to_owned())
            .or_insert_with(|| Loaded {
                body: doc.body.clone(),
                hash: hash(&doc.body),
                last_call: 0,
                receipts: Vec::new(),
            });
        if old.body != doc.body {
            old.body.clone_from(&doc.body);
            old.hash = hash(&doc.body);
            old.receipts.clear();
            self.visible.remove(path);
        }
        old.last_call = self.clock;
    }

    pub fn read_range(
        &mut self,
        path: &str,
        doc: &Document,
        offset: usize,
        bytes: usize,
        content: &str,
    ) {
        let start = offset
            .max(doc.body_offset)
            .saturating_sub(doc.body_offset)
            .min(doc.body.len());
        let end = (offset + bytes)
            .saturating_sub(doc.body_offset)
            .min(doc.body.len());
        self.pending = Some((
            path.to_owned(),
            Receipt {
                call_id: String::new(),
                content: content.to_owned(),
                start,
                end,
            },
        ));
    }

    /// Bind a read range to the exact tool message ultimately sent (spill can alter it).
    pub fn delivered(&mut self, call_id: &str, content: &str) {
        if let Some((path, mut receipt)) = self.pending.take()
            && receipt.content == content
            && let Some(loaded) = self.loaded.get_mut(&path)
        {
            receipt.call_id = call_id.to_owned();
            loaded.receipts.push(receipt);
        }
    }

    pub fn without_preserved(&self, messages: &[Value]) -> Vec<Value> {
        messages
            .iter()
            .filter(|m| self.preserved.as_ref().is_none_or(|p| **m != p.message))
            .cloned()
            .collect()
    }

    /// Uses the same characters/4 estimate as H6, including message/header overhead.
    pub fn preservation(&self, budget: u64) -> Option<Preserved> {
        if !self.enabled {
            return None;
        }
        let budget = budget.min(TOTAL_TOKENS);
        let mut ordered: Vec<_> = self
            .loaded
            .iter()
            .filter(|(path, _)| self.catalog.contains_key(*path))
            .collect();
        ordered.sort_by(|(pa, a), (pb, b)| b.last_call.cmp(&a.last_call).then_with(|| pa.cmp(pb)));
        let mut content = String::from(
            "<retained-skills>\nLatest loaded skill instructions, retained after compaction. For each path these supersede earlier versions.\n",
        );
        let mut entries = BTreeMap::new();
        for (path, loaded) in ordered {
            let name = &self.catalog[path].name;
            let header = format!(
                "\nSkill {} (path: {}; body: {})\n",
                serde_json::to_string(name).unwrap(),
                path,
                loaded.hash
            );
            let chars: Vec<_> = loaded.body.chars().collect();
            let mut lo = 0;
            let mut hi = chars.len().min((PER_SKILL_TOKENS * 4) as usize);
            while lo < hi {
                let mid = lo + (hi - lo).div_ceil(2);
                let prefix: String = chars[..mid].iter().collect();
                if context::estimate(&[json!(prefix)]) <= PER_SKILL_TOKENS {
                    lo = mid;
                } else {
                    hi = mid - 1;
                }
            }
            let maximum = lo;
            let render = |n: usize| {
                let body: String = chars[..n].iter().collect();
                let suffix = if n == chars.len() {
                    "\n"
                } else {
                    "\n[Skill body truncated; read_file the SKILL.md for the full body.]\n"
                };
                json!({"role":"user", "content":format!("{content}{header}{body}{suffix}</retained-skills>")})
            };
            let full_fits = context::estimate(&[render(maximum)]) <= budget;
            let mut lo = if full_fits { maximum } else { 0 };
            let mut hi = if full_fits {
                maximum
            } else {
                maximum.saturating_sub(1)
            };
            while lo < hi {
                let mid = lo + (hi - lo).div_ceil(2);
                if context::estimate(&[render(mid)]) <= budget {
                    lo = mid;
                } else {
                    hi = mid - 1;
                }
            }
            if (lo == 0 && !chars.is_empty()) || context::estimate(&[render(lo)]) > budget {
                break;
            }
            let message = render(lo);
            let text = message["content"].as_str().unwrap();
            content = text.strip_suffix("</retained-skills>").unwrap().to_owned();
            entries.insert(path.clone(), (loaded.hash.clone(), lo == chars.len()));
            if lo < maximum {
                break;
            }
        }
        (!entries.is_empty()).then(|| Preserved {
            message: json!({"role":"user", "content":format!("{content}</retained-skills>")}),
            entries,
        })
    }

    pub fn set_preserved(&mut self, preserved: Option<Preserved>) {
        self.preserved = preserved;
    }

    pub fn validate(&self) -> Result<(), String> {
        for (path, info) in &self.catalog {
            if !valid_key(path) || info.name.trim().is_empty() || info.description.trim().is_empty()
            {
                return Err("invalid saved skill catalog".into());
            }
        }
        for (path, entry) in &self.loaded {
            if !self.catalog.contains_key(path)
                || entry.body.len() as u64 > MAX_FILE_BYTES
                || entry.hash != hash(&entry.body)
                || entry.last_call > self.clock
                || entry
                    .receipts
                    .iter()
                    .any(|r| r.start > r.end || r.end > entry.body.len())
            {
                return Err("invalid saved skill body".into());
            }
        }
        if let Some(p) = &self.preserved
            && (p.message["role"] != "user"
                || !p.message["content"].is_string()
                || p.entries.keys().any(|path| !valid_key(path)))
        {
            return Err("invalid saved skill preservation".into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "skills_tests.rs"]
mod tests;
