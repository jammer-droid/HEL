//! Bounded UTF-8 reads with run-local continuation cursors. A cursor retains the selected
//! range and file identity; it cannot be used to read another run's file or a changed file.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs::{File, Metadata};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::runtime::Runtime;

pub const MAX_BYTES: usize = 10_000;
const CONTENT_BYTES: usize = MAX_BYTES - 160;
const MAX_CURSORS: usize = 128;

#[derive(Default)]
pub struct Reader {
    cursors: RefCell<VecDeque<(String, Position)>>,
}

#[derive(Clone)]
struct Position {
    path: PathBuf,
    offset: u64,
    remaining_lines: Option<u64>,
    identity: Identity,
}

#[derive(Clone, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    length: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl From<Metadata> for Identity {
    fn from(m: Metadata) -> Self {
        Self {
            device: m.dev(),
            inode: m.ino(),
            length: m.len(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
        }
    }
}

pub fn definition() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "read_file",
            "description": "Read UTF-8 text from the project or this run's spill files. start_line is 1-based and max_lines selects a range; omit both to read from the beginning. Returns at most 10000 UTF-8 bytes including any continuation notice. If truncated, call again with only the returned cursor to read the rest of the selected range, including the remainder of a long line. Cursors belong to this run and become invalid if the file changes. Do not guess cursor values.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {"type":"string", "description":"Project-relative or absolute allowed file path. Required unless cursor is provided."},
                    "start_line": {"type":"integer", "minimum":1, "description":"First line, default 1."},
                    "max_lines": {"type":"integer", "minimum":1, "description":"Number of lines; omit to read to EOF within the byte limit."},
                    "cursor": {"type":"string", "description":"An opaque continuation returned by read_file. Use without range arguments."}
                },
                "additionalProperties": false
            }
        }
    })
}

impl Reader {
    pub fn read(&self, runtime: &Runtime, args: &Value) -> Result<String, String> {
        let (mut position, file) = if let Some(cursor) = args.get("cursor") {
            let cursor = cursor.as_str().ok_or("cursor must be a string")?;
            if args.get("start_line").is_some() || args.get("max_lines").is_some() {
                return Err("cursor cannot be combined with start_line or max_lines".into());
            }
            let position = self
                .cursors
                .borrow()
                .iter()
                .find(|(key, _)| key == cursor)
                .map(|(_, p)| p.clone())
                .ok_or("unknown or expired cursor; start a new read")?;
            if let Some(path) = args.get("path") {
                let path = path.as_str().ok_or("path must be a string")?;
                let resolved = runtime
                    .project
                    .join(path)
                    .canonicalize()
                    .map_err(|e| e.to_string())?;
                if resolved != position.path {
                    return Err("cursor belongs to a different file".into());
                }
            }
            let (_, file) = runtime
                .read_open(&position.path)
                .map_err(|e| e.to_string())?;
            if Identity::from(file.metadata().map_err(|e| e.to_string())?) != position.identity {
                return Err("file changed since cursor was created; start a new read".into());
            }
            (position, file)
        } else {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or("missing string argument: path")?;
            let start = positive(args, "start_line")?.unwrap_or(1);
            let remaining_lines = positive(args, "max_lines")?;
            let (path, file) = runtime
                .read_open(Path::new(path))
                .map_err(|e| e.to_string())?;
            let identity = Identity::from(file.metadata().map_err(|e| e.to_string())?);
            let mut reader = BufReader::new(file);
            let mut offset = 0;
            for _ in 1..start {
                let count = reader.skip_until(b'\n').map_err(|e| e.to_string())?;
                if count == 0 {
                    return Err("start_line is beyond end of file".into());
                }
                offset += count as u64;
            }
            (
                Position {
                    path,
                    offset,
                    remaining_lines,
                    identity,
                },
                reader.into_inner(),
            )
        };
        let (content, more, remaining_lines) =
            read_chunk(&file, &position).map_err(|e| e.to_string())?;
        if Identity::from(file.metadata().map_err(|e| e.to_string())?) != position.identity {
            return Err("file changed while reading; start a new read".into());
        }
        position.offset += content.len() as u64;
        position.remaining_lines = remaining_lines;
        if !more {
            return Ok(content);
        }
        let cursor = uuid::Uuid::new_v4().to_string();
        let mut cursors = self.cursors.borrow_mut();
        if cursors.len() == MAX_CURSORS {
            cursors.pop_front();
        }
        cursors.push_back((cursor.clone(), position));
        let response = format!(
            "{content}\n\n[Read truncated. Continue with read_file({{\"cursor\":\"{cursor}\"}}).]\n"
        );
        debug_assert!(response.len() <= MAX_BYTES);
        Ok(response)
    }
}

fn positive(args: &Value, name: &str) -> Result<Option<u64>, String> {
    args.get(name)
        .map(|v| {
            v.as_u64()
                .filter(|n| *n > 0)
                .ok_or_else(|| format!("{name} must be a positive integer"))
        })
        .transpose()
}

fn read_chunk(
    mut file: &File,
    position: &Position,
) -> std::io::Result<(String, bool, Option<u64>)> {
    file.seek(SeekFrom::Start(position.offset))?;
    let mut reader = BufReader::new(file);
    let mut bytes = Vec::with_capacity(CONTENT_BYTES);
    let mut lines = position.remaining_lines;
    while bytes.len() < CONTENT_BYTES && lines != Some(0) {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            break;
        }
        let until_newline = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |i| i + 1);
        let count = until_newline.min(CONTENT_BYTES - bytes.len());
        let newline = available[count - 1] == b'\n';
        bytes.extend_from_slice(&available[..count]);
        reader.consume(count);
        if newline {
            lines = lines.map(|n| n - 1);
        }
    }
    let more = lines != Some(0) && !reader.fill_buf()?.is_empty();
    let content = match std::str::from_utf8(&bytes) {
        Ok(text) => text.to_owned(),
        Err(e) if e.error_len().is_none() && more => {
            // Retry the incomplete trailing code point at the next byte offset.
            std::str::from_utf8(&bytes[..e.valid_up_to()])
                .unwrap()
                .to_owned()
        }
        Err(e) => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
    };
    Ok((content, more, lines))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup() -> (PathBuf, Runtime) {
        let root = std::env::temp_dir().join(format!("hel-reader-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let runtime = Runtime::new_in(&root, &root.join("runs")).unwrap();
        (root, runtime)
    }

    fn split(response: &str) -> (&str, Option<String>) {
        if let Some((text, notice)) = response.split_once("\n\n[Read truncated.") {
            let cursor = notice
                .split("\"cursor\":\"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap();
            (text, Some(cursor.to_owned()))
        } else {
            (response, None)
        }
    }

    #[test]
    fn line_ranges_and_long_utf8_line_continue_without_loss() {
        let (root, runtime) = setup();
        let selected = format!("{}\nlast selected\n", "한글🙂".repeat(4000));
        fs::write(root.join("text"), format!("skip\n{selected}not selected\n")).unwrap();
        let mut args = json!({"path":"text","start_line":2,"max_lines":2});
        let mut joined = String::new();
        let mut chunks = 0;
        loop {
            let response = runtime.reader.read(&runtime, &args).unwrap();
            assert!(response.len() <= MAX_BYTES);
            let (text, cursor) = split(&response);
            joined.push_str(text);
            chunks += 1;
            match cursor {
                Some(cursor) => args = json!({"cursor":cursor}),
                None => break,
            }
            assert!(chunks < 10);
        }
        assert!(chunks > 1);
        assert_eq!(joined, selected);
        assert_eq!(
            runtime
                .reader
                .read(&runtime, &json!({"path":"text","max_lines":1}))
                .unwrap(),
            "skip\n"
        );
        drop(runtime);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn default_reads_are_bounded_and_cursors_reject_changes_and_other_runs() {
        let (root, runtime) = setup();
        fs::write(root.join("text"), "x".repeat(30_000)).unwrap();
        let first = runtime
            .reader
            .read(&runtime, &json!({"path":"text"}))
            .unwrap();
        assert!(first.len() <= MAX_BYTES);
        let cursor = split(&first).1.unwrap();
        let other = Runtime::new_in(&root, &root.join("runs")).unwrap();
        assert!(
            other
                .reader
                .read(&other, &json!({"cursor":cursor}))
                .unwrap_err()
                .contains("unknown")
        );
        assert!(
            runtime
                .reader
                .read(&runtime, &json!({"cursor":cursor,"start_line":1}))
                .is_err()
        );
        fs::write(root.join("text"), "changed").unwrap();
        assert!(
            runtime
                .reader
                .read(&runtime, &json!({"cursor":cursor}))
                .unwrap_err()
                .contains("changed")
        );
        for args in [
            json!({"path":"text","start_line":0}),
            json!({"path":"text","max_lines":-1}),
            json!({"path":"text","max_lines":"2"}),
        ] {
            assert!(runtime.reader.read(&runtime, &args).is_err());
        }
        fs::write(root.join("invalid"), [0xff]).unwrap();
        assert!(
            runtime
                .reader
                .read(&runtime, &json!({"path":"invalid"}))
                .is_err()
        );
        drop(other);
        drop(runtime);
        fs::remove_dir_all(root).unwrap();
    }
}
