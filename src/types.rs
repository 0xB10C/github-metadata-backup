use chrono::{DateTime, Utc};
use clap::Parser;
use octocrab::models::{issues, pulls, timelines};
use serde::{Deserialize, Serialize};
use std::error;
use std::fmt;
use std::io;
use std::path::PathBuf;

#[derive(Debug)]
pub enum WriteError {
    IoError(io::Error),
    JsonSerdeError(serde_json::Error),
}

impl From<io::Error> for WriteError {
    fn from(err: io::Error) -> Self {
        WriteError::IoError(err)
    }
}

impl From<serde_json::Error> for WriteError {
    fn from(err: serde_json::Error) -> Self {
        WriteError::JsonSerdeError(err)
    }
}

impl error::Error for WriteError {}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            WriteError::IoError(e) => write!(f, "WriteError::IoError: {}", e),
            WriteError::JsonSerdeError(e) => write!(f, "WriteError::JsonSerdeError: {}", e),
        }
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Owner of the repository to backup
    #[arg(short, long)]
    pub owner: String,
    /// Name of the repository to backup
    #[arg(short, long)]
    pub repo: String,
    /// Personal Access Token to the GitHub API
    #[arg(short, long)]
    pub personal_access_token: String,
    /// Destination where the backup should be written to
    #[arg(short, long, value_name = "PATH")]
    pub destination: PathBuf,
}

#[derive(Debug, Clone)]
pub enum EntryWithMetadata {
    Issue(IssueWithMetadata),
    Pull(PullWithMetadata),
}

impl fmt::Display for EntryWithMetadata {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            EntryWithMetadata::Issue(i) => write!(f, "issue #{}", i.issue.number),
            EntryWithMetadata::Pull(p) => write!(f, "pull-request #{}", p.pull.number),
        }
    }
}

/// A GitHub Issue with metadata. Can be serialized.
#[derive(Serialize, Debug, Clone)]
pub struct IssueWithMetadata {
    pub r#type: String,
    pub issue: issues::Issue,
    pub events: Vec<timelines::TimelineEvent>,
}

impl IssueWithMetadata {
    pub fn new(issue: issues::Issue, events: Vec<timelines::TimelineEvent>) -> Self {
        Self {
            r#type: "issue".to_string(),
            issue,
            events,
        }
    }
}

/// A GitHub Pull-Request with metadata. Can be serialized.
#[derive(Serialize, Debug, Clone)]
pub struct PullWithMetadata {
    pub r#type: String,
    pub pull: pulls::PullRequest,
    pub events: Vec<timelines::TimelineEvent>,
    pub comments: Vec<pulls::Comment>,
}

impl PullWithMetadata {
    pub fn new(
        pull: pulls::PullRequest,
        events: Vec<timelines::TimelineEvent>,
        comments: Vec<pulls::Comment>,
    ) -> Self {
        Self {
            r#type: "pull".to_string(),
            pull,
            events,
            comments,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BackupState {
    /// Version of the BackupState
    pub version: u32,
    /// UTC Unix timestamp when the last backup was completed.
    pub last_backup: DateTime<Utc>,
}
