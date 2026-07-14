//! Privacy-preserving, read-only bridges to external coding-agent sessions.
//!
//! Providers expose session identity and lifecycle signals only. Prompt and
//! response bodies never leave the agent-owned transcript parser.

mod claude;

use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use tokio::task::JoinHandle;

pub use claude::{
    parse_claude_transcript_record, ClaudeProvider, ClaudeProviderOptions, ClaudeTranscriptSignal,
};

/// Metadata needed to identify and watch a coding-agent session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSession {
    pub id: String,
    pub project: String,
    pub title: String,
    pub transcript: PathBuf,
    pub updated_at: SystemTime,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionQuery {
    pub query: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionReason {
    TurnDuration,
}

/// Lifecycle-only signals emitted by an external coding-agent session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEvent {
    Watching {
        session_id: String,
        at: SystemTime,
    },
    TurnStarted {
        session_id: String,
        at: SystemTime,
        prompt_id: Option<String>,
    },
    TurnCompleted {
        session_id: String,
        at: SystemTime,
        reason: CompletionReason,
    },
    Interrupted {
        session_id: String,
        at: SystemTime,
    },
    Error {
        session_id: String,
        at: SystemTime,
        message: String,
    },
}

#[derive(Debug)]
pub enum AgentError {
    TranscriptUnavailable(std::io::Error),
    RuntimeUnavailable,
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TranscriptUnavailable(error) => {
                write!(
                    formatter,
                    "agent session transcript is unavailable: {error}"
                )
            }
            Self::RuntimeUnavailable => {
                formatter.write_str("agent session could not be watched outside a Tokio runtime")
            }
        }
    }
}

impl std::error::Error for AgentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::TranscriptUnavailable(error) => Some(error),
            Self::RuntimeUnavailable => None,
        }
    }
}

/// A cancellable, bounded lifecycle event stream.
pub struct AgentWatch {
    receiver: flume::Receiver<AgentEvent>,
    shutdown: Arc<AtomicBool>,
    task: JoinHandle<()>,
}

impl AgentWatch {
    pub(crate) fn new(
        receiver: flume::Receiver<AgentEvent>,
        shutdown: Arc<AtomicBool>,
        task: JoinHandle<()>,
    ) -> Self {
        Self {
            receiver,
            shutdown,
            task,
        }
    }

    pub async fn recv(&mut self) -> Option<AgentEvent> {
        self.receiver.recv_async().await.ok()
    }

    pub fn try_recv(&mut self) -> Result<AgentEvent, flume::TryRecvError> {
        self.receiver.try_recv()
    }

    pub async fn stop(mut self) {
        self.shutdown.store(true, Ordering::Release);
        self.task.abort();
        let _ = (&mut self.task).await;
    }
}

impl Drop for AgentWatch {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        self.task.abort();
    }
}

#[async_trait]
pub trait AgentProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;

    async fn list_sessions(&self, query: SessionQuery) -> Result<Vec<AgentSession>, AgentError>;

    /// Observe only future records. Existing history is skipped at current EOF.
    async fn watch(&self, session: AgentSession) -> Result<AgentWatch, AgentError>;
}

#[derive(Debug, Clone)]
pub(crate) struct WatchOptions {
    pub poll_interval: Duration,
    pub max_line_bytes: usize,
    pub channel_capacity: usize,
}
