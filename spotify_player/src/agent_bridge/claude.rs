use std::cmp::Reverse;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use serde_json::{Map, Value};

use super::{
    AgentError, AgentEvent, AgentProvider, AgentSession, AgentWatch, CompletionReason,
    SessionQuery, WatchOptions,
};

const DEFAULT_LIMIT: usize = 25;
const MAX_LIMIT: usize = 100;
const DEFAULT_METADATA_BYTES: usize = 256 * 1024;
const DEFAULT_MAX_CANDIDATES: usize = 300;
const DEFAULT_MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ClaudeProviderOptions {
    pub projects_root: Option<PathBuf>,
    pub poll_interval: Duration,
    pub metadata_bytes: usize,
    pub max_candidates: usize,
    pub max_line_bytes: usize,
    pub channel_capacity: usize,
}

impl Default for ClaudeProviderOptions {
    fn default() -> Self {
        Self {
            projects_root: None,
            poll_interval: Duration::from_millis(250),
            metadata_bytes: DEFAULT_METADATA_BYTES,
            max_candidates: DEFAULT_MAX_CANDIDATES,
            max_line_bytes: DEFAULT_MAX_LINE_BYTES,
            channel_capacity: 64,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClaudeProvider {
    projects_root: PathBuf,
    metadata_bytes: usize,
    max_candidates: usize,
    watch: WatchOptions,
}

impl Default for ClaudeProvider {
    fn default() -> Self {
        Self::new(ClaudeProviderOptions::default())
    }
}

impl ClaudeProvider {
    pub fn new(options: ClaudeProviderOptions) -> Self {
        let projects_root = options.projects_root.unwrap_or_else(default_projects_root);
        Self {
            projects_root,
            metadata_bytes: options.metadata_bytes.max(16 * 1024),
            max_candidates: options.max_candidates.max(1),
            watch: WatchOptions {
                poll_interval: options.poll_interval.max(Duration::from_millis(25)),
                max_line_bytes: options.max_line_bytes.max(64 * 1024),
                channel_capacity: options.channel_capacity.max(4),
            },
        }
    }

    #[must_use]
    pub fn projects_root(&self) -> &Path {
        &self.projects_root
    }
}

#[async_trait]
impl AgentProvider for ClaudeProvider {
    fn id(&self) -> &'static str {
        "claude"
    }

    fn display_name(&self) -> &'static str {
        "Claude Code"
    }

    async fn list_sessions(&self, query: SessionQuery) -> Result<Vec<AgentSession>, AgentError> {
        let mut candidates = find_candidates(&self.projects_root);
        candidates.sort_by_key(|candidate| Reverse(candidate.updated_at));

        let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let needle = query
            .query
            .as_deref()
            .map(str::trim)
            .filter(|query| !query.is_empty());
        let scan_limit = if needle.is_some() {
            self.max_candidates
        } else {
            self.max_candidates.min((limit * 4).max(40))
        };

        let mut sessions = Vec::with_capacity(limit);
        for candidate in candidates.into_iter().take(scan_limit) {
            let Some(session) = session_from_candidate(candidate, self.metadata_bytes) else {
                continue;
            };
            if let Some(needle) = needle {
                if !session_matches(&session, needle) {
                    continue;
                }
            }
            sessions.push(session);
            if sessions.len() == limit {
                break;
            }
        }
        Ok(sessions)
    }

    async fn watch(&self, session: AgentSession) -> Result<AgentWatch, AgentError> {
        let metadata =
            fs::metadata(&session.transcript).map_err(AgentError::TranscriptUnavailable)?;
        let initial = TailPosition {
            identity: FileIdentity::from_metadata(&metadata),
            offset: metadata.len(),
            anchor: read_anchor(&session.transcript, metadata.len())
                .map_err(AgentError::TranscriptUnavailable)?,
        };
        let (sender, receiver) = flume::bounded(self.watch.channel_capacity);
        let shutdown = Arc::new(AtomicBool::new(false));
        let task = tokio::runtime::Handle::try_current()
            .map_err(|_| AgentError::RuntimeUnavailable)?
            .spawn(run_tail(
                session,
                initial,
                self.watch.clone(),
                sender,
                Arc::clone(&shutdown),
            ));
        Ok(AgentWatch::new(receiver, shutdown, task))
    }
}

fn default_projects_root() -> PathBuf {
    dirs_next::home_dir().map_or_else(
        || PathBuf::from(".claude/projects"),
        |home| home.join(".claude/projects"),
    )
}

#[derive(Debug)]
struct Candidate {
    path: PathBuf,
    updated_at: SystemTime,
}

fn find_candidates(root: &Path) -> Vec<Candidate> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];

    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if entry.file_name() != OsStr::new("subagents") {
                    pending.push(entry.path());
                }
                continue;
            }
            if !file_type.is_file() || entry.path().extension() != Some(OsStr::new("jsonl")) {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            found.push(Candidate {
                path: entry.path(),
                updated_at: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            });
        }
    }
    found
}

fn session_from_candidate(candidate: Candidate, read_limit: usize) -> Option<AgentSession> {
    let mut file = File::open(&candidate.path).ok()?.take(read_limit as u64);
    let mut bytes = Vec::with_capacity(read_limit.min(64 * 1024));
    file.read_to_end(&mut bytes).ok()?;

    let mut id = None;
    let mut cwd = None;
    for line in bytes.split(|byte| *byte == b'\n') {
        let Ok(Value::Object(record)) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        id = id.or_else(|| string_field(&record, "sessionId").map(ToOwned::to_owned));
        cwd = cwd.or_else(|| string_field(&record, "cwd").map(PathBuf::from));
        if id.is_some() && cwd.is_some() {
            break;
        }
    }

    let transcript = candidate.path;
    let fallback_id = transcript.file_stem()?.to_string_lossy().into_owned();
    let cwd = cwd.unwrap_or_else(|| {
        transcript
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    });
    if !fs::metadata(&cwd).is_ok_and(|metadata| metadata.is_dir()) {
        return None;
    }
    let project = cwd
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| cwd.display().to_string());

    Some(AgentSession {
        id: id.unwrap_or(fallback_id),
        title: project.clone(),
        project,
        transcript,
        updated_at: candidate.updated_at,
    })
}

fn session_matches(session: &AgentSession, needle: &str) -> bool {
    let needle = needle.to_lowercase();
    [
        session.id.as_str(),
        session.project.as_str(),
        session.title.as_str(),
        session.transcript.to_str().unwrap_or_default(),
    ]
    .iter()
    .any(|field| field.to_lowercase().contains(&needle))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeTranscriptSignal {
    TurnStarted {
        at: SystemTime,
        prompt_id: Option<String>,
    },
    TurnCompleted {
        at: SystemTime,
    },
    Interrupted {
        at: SystemTime,
    },
}

/// Parses a record into lifecycle information without returning any content.
pub fn parse_claude_transcript_record(line: &[u8]) -> Option<ClaudeTranscriptSignal> {
    let Value::Object(record) = serde_json::from_slice::<Value>(line).ok()? else {
        return None;
    };
    if ignored_record(&record) {
        return None;
    }

    let at = record_time(&record);
    if is_interrupted(&record) {
        return Some(ClaudeTranscriptSignal::Interrupted { at });
    }
    if is_real_prompt(&record) {
        let prompt_id = string_field(&record, "promptId")
            .or_else(|| string_field(&record, "uuid"))
            .map(ToOwned::to_owned);
        return Some(ClaudeTranscriptSignal::TurnStarted { at, prompt_id });
    }
    if string_field(&record, "type") == Some("system")
        && string_field(&record, "subtype") == Some("turn_duration")
    {
        return Some(ClaudeTranscriptSignal::TurnCompleted { at });
    }
    None
}

fn ignored_record(record: &Map<String, Value>) -> bool {
    bool_field(record, "isMeta")
        || bool_field(record, "isSidechain")
        || bool_field(record, "isSynthetic")
        || bool_field(record, "synthetic")
        || record
            .get("sourceToolAssistantUUID")
            .is_some_and(|value| !value.is_null())
        || string_field(record, "promptSource") == Some("sdk")
        || string_field(record, "entrypoint") == Some("sdk-cli")
        || record
            .get("message")
            .and_then(Value::as_object)
            .and_then(|message| message.get("model"))
            .and_then(Value::as_str)
            == Some("<synthetic>")
}

fn is_real_prompt(record: &Map<String, Value>) -> bool {
    if string_field(record, "type") != Some("user")
        || !matches!(
            string_field(record, "promptSource"),
            Some("typed" | "queued")
        )
    {
        return false;
    }
    let Some(message) = record.get("message").and_then(Value::as_object) else {
        return false;
    };
    if string_field(message, "role") != Some("user") || has_content_type(message, "tool_result") {
        return false;
    }
    let text = message_text(message);
    let text = text.trim();
    !text.is_empty()
        && !text.starts_with("<local-command-")
        && !text.starts_with("<command-name>")
        && !text.starts_with("<task-notification>")
        && !text.starts_with("[Request interrupted by user]")
}

fn is_interrupted(record: &Map<String, Value>) -> bool {
    string_field(record, "type") == Some("user")
        && record
            .get("message")
            .and_then(Value::as_object)
            .is_some_and(|message| {
                message_text(message)
                    .trim_start()
                    .starts_with("[Request interrupted by user]")
            })
}

fn message_text(message: &Map<String, Value>) -> String {
    match message.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(Value::as_object)
            .filter(|part| string_field(part, "type") == Some("text"))
            .filter_map(|part| string_field(part, "text"))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn has_content_type(message: &Map<String, Value>, expected: &str) -> bool {
    message
        .get("content")
        .and_then(Value::as_array)
        .is_some_and(|parts| {
            parts
                .iter()
                .filter_map(Value::as_object)
                .any(|part| string_field(part, "type") == Some(expected))
        })
}

fn string_field<'a>(record: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    record
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn bool_field(record: &Map<String, Value>, key: &str) -> bool {
    record.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn record_time(record: &Map<String, Value>) -> SystemTime {
    string_field(record, "timestamp")
        .and_then(|timestamp| chrono::DateTime::parse_from_rfc3339(timestamp).ok())
        .map_or_else(SystemTime::now, SystemTime::from)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    first: u64,
    second: u64,
}

impl FileIdentity {
    #[cfg(unix)]
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            first: metadata.dev(),
            second: metadata.ino(),
        }
    }

    #[cfg(not(unix))]
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        let created = metadata
            .created()
            .ok()
            .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0);
        Self {
            first: created,
            second: 0,
        }
    }
}

#[derive(Debug, Clone)]
struct TailPosition {
    identity: FileIdentity,
    offset: u64,
    anchor: Vec<u8>,
}

#[derive(Default)]
struct LineBuffer {
    pending: Vec<u8>,
    discarding_oversized: bool,
}

impl LineBuffer {
    fn reset(&mut self) {
        self.pending.clear();
        self.discarding_oversized = false;
    }

    fn consume(&mut self, bytes: &[u8], max_line_bytes: usize) -> Vec<Vec<u8>> {
        let mut lines = Vec::new();
        for byte in bytes {
            if *byte == b'\n' {
                if !self.discarding_oversized && !self.pending.is_empty() {
                    lines.push(std::mem::take(&mut self.pending));
                } else {
                    self.pending.clear();
                }
                self.discarding_oversized = false;
            } else if !self.discarding_oversized {
                if self.pending.len() >= max_line_bytes {
                    self.pending.clear();
                    self.discarding_oversized = true;
                } else {
                    self.pending.push(*byte);
                }
            }
        }
        lines
    }
}

async fn run_tail(
    session: AgentSession,
    mut position: TailPosition,
    options: WatchOptions,
    sender: flume::Sender<AgentEvent>,
    shutdown: Arc<AtomicBool>,
) {
    if sender
        .send_async(AgentEvent::Watching {
            session_id: session.id.clone(),
            at: SystemTime::now(),
        })
        .await
        .is_err()
    {
        return;
    }

    let mut interval = tokio::time::interval(options.poll_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut lines = LineBuffer::default();
    let mut active = false;
    let mut last_error = String::new();

    while !shutdown.load(Ordering::Acquire) {
        interval.tick().await;
        match poll_file(
            &session.transcript,
            &mut position,
            &mut lines,
            options.max_line_bytes,
        ) {
            Ok(signals) => {
                last_error.clear();
                for signal in signals {
                    let event = match signal {
                        ClaudeTranscriptSignal::TurnStarted { at, prompt_id } if !active => {
                            active = true;
                            Some(AgentEvent::TurnStarted {
                                session_id: session.id.clone(),
                                at,
                                prompt_id,
                            })
                        }
                        ClaudeTranscriptSignal::TurnCompleted { at } if active => {
                            active = false;
                            Some(AgentEvent::TurnCompleted {
                                session_id: session.id.clone(),
                                at,
                                reason: CompletionReason::TurnDuration,
                            })
                        }
                        ClaudeTranscriptSignal::Interrupted { at } if active => {
                            active = false;
                            Some(AgentEvent::Interrupted {
                                session_id: session.id.clone(),
                                at,
                            })
                        }
                        _ => None,
                    };
                    if let Some(event) = event {
                        if sender.send_async(event).await.is_err() {
                            return;
                        }
                    }
                }
            }
            Err(error) => {
                let message = match error.kind() {
                    std::io::ErrorKind::NotFound => "Claude transcript unavailable (not found).",
                    std::io::ErrorKind::PermissionDenied => {
                        "Claude transcript unavailable (permission denied)."
                    }
                    _ => "Claude transcript could not be read.",
                }
                .to_owned();
                if message != last_error {
                    last_error.clone_from(&message);
                    if sender
                        .send_async(AgentEvent::Error {
                            session_id: session.id.clone(),
                            at: SystemTime::now(),
                            message,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
    }
}

fn poll_file(
    path: &Path,
    position: &mut TailPosition,
    lines: &mut LineBuffer,
    max_line_bytes: usize,
) -> std::io::Result<Vec<ClaudeTranscriptSignal>> {
    let metadata = fs::metadata(path)?;
    let identity = FileIdentity::from_metadata(&metadata);
    if identity != position.identity {
        position.identity = identity;
        position.offset = metadata.len();
        position.anchor = read_anchor(path, position.offset)?;
        lines.reset();
        return Ok(Vec::new());
    }
    if metadata.len() < position.offset {
        position.offset = metadata.len();
        position.anchor = read_anchor(path, position.offset)?;
        lines.reset();
        return Ok(Vec::new());
    }
    if !anchor_is_current(path, position)? {
        position.offset = metadata.len();
        position.anchor = read_anchor(path, position.offset)?;
        lines.reset();
        return Ok(Vec::new());
    }
    if metadata.len() == position.offset {
        return Ok(Vec::new());
    }

    let mut file = File::open(path)?;
    let opened_metadata = file.metadata()?;
    let opened_identity = FileIdentity::from_metadata(&opened_metadata);
    if opened_identity != identity {
        position.identity = opened_identity;
        position.offset = opened_metadata.len();
        position.anchor = read_anchor(path, position.offset)?;
        lines.reset();
        return Ok(Vec::new());
    }
    file.seek(SeekFrom::Start(position.offset))?;
    let mut chunk = vec![0; 64 * 1024];
    let mut signals = Vec::new();
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        position.offset += read as u64;
        signals.extend(
            lines
                .consume(&chunk[..read], max_line_bytes)
                .into_iter()
                .filter_map(|line| parse_claude_transcript_record(&line)),
        );
    }
    position.anchor = read_anchor(path, position.offset)?;
    Ok(signals)
}

const ANCHOR_BYTES: u64 = 128;

fn read_anchor(path: &Path, offset: u64) -> std::io::Result<Vec<u8>> {
    let length = offset.min(ANCHOR_BYTES);
    if length == 0 {
        return Ok(Vec::new());
    }
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset - length))?;
    let mut anchor = vec![0; length as usize];
    file.read_exact(&mut anchor)?;
    Ok(anchor)
}

fn anchor_is_current(path: &Path, position: &TailPosition) -> std::io::Result<bool> {
    if position.anchor.is_empty() {
        return Ok(true);
    }
    let start = position.offset.saturating_sub(position.anchor.len() as u64);
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut current = vec![0; position.anchor.len()];
    file.read_exact(&mut current)?;
    Ok(current == position.anchor)
}

#[cfg(test)]
#[allow(clippy::needless_pass_by_value)]
mod tests {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    use tokio::time::timeout;

    use super::*;

    static TEMP_ID: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("kill-silence-agent-{}-{id}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn json_line(value: Value) -> String {
        format!("{}\n", serde_json::to_string(&value).unwrap())
    }

    fn prompt(source: &str, content: Value) -> Value {
        serde_json::json!({
            "type": "user",
            "promptSource": source,
            "uuid": "prompt-1",
            "timestamp": "2026-07-14T12:00:00Z",
            "message": { "role": "user", "content": content }
        })
    }

    fn test_session(path: PathBuf) -> AgentSession {
        AgentSession {
            id: "session-1".into(),
            project: "demo".into(),
            title: "demo".into(),
            transcript: path,
            updated_at: SystemTime::now(),
        }
    }

    fn provider(root: &Path) -> ClaudeProvider {
        ClaudeProvider::new(ClaudeProviderOptions {
            projects_root: Some(root.to_path_buf()),
            poll_interval: Duration::from_millis(25),
            ..ClaudeProviderOptions::default()
        })
    }

    #[test]
    fn parser_accepts_only_typed_or_queued_human_prompts() {
        for source in ["typed", "queued"] {
            assert!(matches!(
                parse_claude_transcript_record(
                    json_line(prompt(source, Value::String("secret".into()))).as_bytes()
                ),
                Some(ClaudeTranscriptSignal::TurnStarted { .. })
            ));
        }
        for source in ["sdk", "tool", "bridge"] {
            assert_eq!(
                parse_claude_transcript_record(
                    json_line(prompt(source, Value::String("secret".into()))).as_bytes()
                ),
                None
            );
        }
        let tool_result = prompt(
            "typed",
            serde_json::json!([{"type":"tool_result","content":"secret"}]),
        );
        assert_eq!(
            parse_claude_transcript_record(json_line(tool_result).as_bytes()),
            None
        );
    }

    #[test]
    fn parser_exposes_lifecycle_only_and_ignores_internal_records() {
        let interrupted = prompt(
            "typed",
            Value::String("[Request interrupted by user] private".into()),
        );
        assert!(matches!(
            parse_claude_transcript_record(json_line(interrupted).as_bytes()),
            Some(ClaudeTranscriptSignal::Interrupted { .. })
        ));
        let completed = serde_json::json!({"type":"system","subtype":"turn_duration"});
        assert!(matches!(
            parse_claude_transcript_record(json_line(completed).as_bytes()),
            Some(ClaudeTranscriptSignal::TurnCompleted { .. })
        ));
        let assistant =
            serde_json::json!({"type":"assistant","message":{"content":"private response"}});
        assert_eq!(
            parse_claude_transcript_record(json_line(assistant).as_bytes()),
            None
        );
        for flag in ["isMeta", "isSidechain", "isSynthetic", "synthetic"] {
            let mut value = prompt("typed", Value::String("secret".into()));
            value
                .as_object_mut()
                .unwrap()
                .insert(flag.into(), Value::Bool(true));
            assert_eq!(
                parse_claude_transcript_record(json_line(value).as_bytes()),
                None
            );
        }
    }

    #[tokio::test]
    async fn sessions_are_recursive_recent_first_and_exclude_subagents() {
        let temp = TempDir::new();
        let cwd = temp.path().join("workspace");
        std::fs::create_dir(&cwd).unwrap();
        let old_dir = temp.path().join("project-a");
        let new_dir = temp.path().join("project-b");
        let subagents = new_dir.join("subagents");
        std::fs::create_dir_all(&old_dir).unwrap();
        std::fs::create_dir_all(&subagents).unwrap();
        std::fs::write(
            old_dir.join("old.jsonl"),
            json_line(serde_json::json!({"sessionId":"old","cwd":cwd})),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(
            new_dir.join("new.jsonl"),
            json_line(serde_json::json!({"sessionId":"new","cwd":cwd})),
        )
        .unwrap();
        std::fs::write(
            subagents.join("hidden.jsonl"),
            json_line(serde_json::json!({"sessionId":"hidden","cwd":cwd})),
        )
        .unwrap();

        let sessions = provider(temp.path())
            .list_sessions(SessionQuery::default())
            .await
            .unwrap();
        assert_eq!(
            sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            ["new", "old"]
        );
        assert!(sessions.iter().all(|session| session.title == "workspace"));
    }

    #[tokio::test]
    async fn watcher_starts_at_eof_and_handles_partial_jsonl() {
        let temp = TempDir::new();
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            json_line(prompt("typed", Value::String("historical".into()))),
        )
        .unwrap();
        let mut watcher = provider(temp.path())
            .watch(test_session(path.clone()))
            .await
            .unwrap();
        assert!(matches!(
            watcher.recv().await,
            Some(AgentEvent::Watching { .. })
        ));

        let fresh = json_line(prompt("typed", Value::String("fresh but private".into())));
        let midpoint = fresh.len() / 2;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(&fresh.as_bytes()[..midpoint]).unwrap();
        file.flush().unwrap();
        assert!(timeout(Duration::from_millis(80), watcher.recv())
            .await
            .is_err());
        file.write_all(&fresh.as_bytes()[midpoint..]).unwrap();
        file.flush().unwrap();

        assert!(matches!(
            timeout(Duration::from_secs(1), watcher.recv())
                .await
                .unwrap()
                .unwrap(),
            AgentEvent::TurnStarted { prompt_id: Some(ref id), .. } if id == "prompt-1"
        ));
        file.write_all(
            json_line(serde_json::json!({"type":"system","subtype":"turn_duration"})).as_bytes(),
        )
        .unwrap();
        file.flush().unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(1), watcher.recv())
                .await
                .unwrap()
                .unwrap(),
            AgentEvent::TurnCompleted { .. }
        ));
    }

    #[tokio::test]
    async fn watcher_skips_rewritten_history_after_truncate() {
        let temp = TempDir::new();
        let path = temp.path().join("session.jsonl");
        std::fs::write(&path, "{}\n{}\n").unwrap();
        let mut watcher = provider(temp.path())
            .watch(test_session(path.clone()))
            .await
            .unwrap();
        let _ = watcher.recv().await;

        std::fs::write(
            &path,
            json_line(prompt("typed", Value::String("rewritten history".into()))),
        )
        .unwrap();
        assert!(timeout(Duration::from_millis(100), watcher.recv())
            .await
            .is_err());
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(json_line(prompt("queued", Value::String("future".into()))).as_bytes())
            .unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(1), watcher.recv())
                .await
                .unwrap()
                .unwrap(),
            AgentEvent::TurnStarted { .. }
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn watcher_skips_replacement_file_history() {
        let temp = TempDir::new();
        let path = temp.path().join("session.jsonl");
        std::fs::write(&path, "{}\n").unwrap();
        let mut watcher = provider(temp.path())
            .watch(test_session(path.clone()))
            .await
            .unwrap();
        let _ = watcher.recv().await;

        let replacement = temp.path().join("replacement.jsonl");
        std::fs::write(
            &replacement,
            json_line(prompt("typed", Value::String("replacement history".into()))),
        )
        .unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        assert!(timeout(Duration::from_millis(100), watcher.recv())
            .await
            .is_err());
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(json_line(prompt("queued", Value::String("future".into()))).as_bytes())
            .unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(1), watcher.recv())
                .await
                .unwrap()
                .unwrap(),
            AgentEvent::TurnStarted { .. }
        ));
    }
}
