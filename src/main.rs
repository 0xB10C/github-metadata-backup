use async_recursion::async_recursion;
use chrono::prelude::*;
use clap::Parser;
use env_logger::Env;
use log::{debug, error, info, warn};
use octocrab::models::{issues, pulls};
use octocrab::Page;
use octocrab::{models, params};
use std::fs;
use std::fs::File;
use std::io::prelude::*;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::SystemTime;
use tokio::sync::mpsc;
use tokio::task;
use tokio::time::{sleep, Duration};

use types::*;

const STATE_FILE_PATH: &str = "state.json";

const MAX_PER_PAGE: u8 = 100;
const START_PAGE: u32 = 1; // GitHub starts indexing at page 1
const STATE_VERSION: u32 = 1;

const EXIT_CREATING_DIRS: u8 = 1;
const EXIT_CREATING_OCTOCRAB_INSTANCE: u8 = 2;
const EXIT_API_ERROR: u8 = 3;
const EXIT_WRITING: u8 = 3;
const EXIT_NO_PAT: u8 = 4;

mod types;

async fn wait_on_ratelimit() {
    let gh = octocrab::instance();
    let now = SystemTime::now();
    let unix_time = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("SystemTime before UNIX EPOCH!")
        .as_secs();

    loop {
        let ratelimit = gh
            .ratelimit()
            .get()
            .await
            .expect("could not get ratelimit info");
        let remaining = ratelimit.resources.core.remaining;

        if remaining > 0 {
            break;
        }

        let reset = ratelimit.resources.core.reset;
        let reset_in = (reset - unix_time as usize) + 2;

        info!(
            "GitHub rate-limit hit (remaining={}): should reset in {} seconds (at {}).",
            remaining, reset_in, reset
        );
        info!("Waiting..");
        sleep(Duration::from_secs(reset_in as u64)).await;
    }
    info!("Github rate-limiting has reset.");
}

#[async_recursion]
async fn get_pull_body(
    number: u64,
    owner: String,
    repo: String,
    attempt: u8,
) -> octocrab::Result<pulls::PullRequest> {
    match octocrab::instance()
        .pulls(owner.clone(), repo.clone())
        .get(number)
        .await
    {
        Ok(p) => Ok(p),
        Err(e) => {
            match e {
                octocrab::Error::GitHub { .. } => {
                    if attempt > 0 {
                        return Err(e);
                    }
                    // retry once incase we hit the rate-limiting
                    wait_on_ratelimit().await;
                    get_pull_body(number, owner, repo, attempt + 1).await
                }
                _ => Err(e),
            }
        }
    }
}

#[async_recursion]
async fn get_pull_comments_page(
    number: u64,
    page: u32,
    owner: String,
    repo: String,
    attempt: u8,
) -> octocrab::Result<Page<pulls::Comment>> {
    match octocrab::instance()
        .pulls(owner.clone(), repo.clone())
        .list_comments(Some(number))
        .per_page(MAX_PER_PAGE)
        .page(page)
        .send()
        .await
    {
        Ok(p) => Ok(p),
        Err(e) => {
            match e {
                octocrab::Error::GitHub { .. } => {
                    if attempt > 0 {
                        return Err(e);
                    }
                    // retry once incase we hit the rate-limiting
                    wait_on_ratelimit().await;
                    get_pull_comments_page(number, page, owner, repo, attempt + 1).await
                }
                _ => Err(e),
            }
        }
    }
}

async fn get_pull_comments(
    number: u64,
    owner: String,
    repo: String,
) -> Result<Vec<models::pulls::Comment>, octocrab::Error> {
    let mut comments = Vec::<models::pulls::Comment>::new();

    for page in 1..u32::MAX {
        match get_pull_comments_page(number, page, owner.clone(), repo.clone(), 0).await {
            Ok(mut comments_page) => {
                comments.append(&mut comments_page.take_items());

                debug!(
                    "Loaded {} comments for pull {} in {}:{}",
                    comments.len(),
                    number,
                    owner,
                    repo
                );

                if comments_page.next.is_none() {
                    return Ok(comments);
                }
            }
            Err(e) => return Err(e),
        }
    }

    Ok(comments)
}

#[async_recursion]
async fn get_timeline_page(
    number: u64,
    page: u32,
    owner: String,
    repo: String,
    attempt: u8,
) -> octocrab::Result<Page<octocrab::models::timelines::TimelineEvent>> {
    match octocrab::instance()
        .issues(owner.clone(), repo.clone())
        .list_timeline_events(number)
        .per_page(MAX_PER_PAGE)
        .page(page)
        .send()
        .await
    {
        Ok(p) => Ok(p),
        Err(e) => {
            match e {
                octocrab::Error::GitHub { .. } => {
                    if attempt > 0 {
                        return Err(e);
                    }
                    // retry once incase we hit the rate-limiting
                    wait_on_ratelimit().await;
                    get_timeline_page(number, page, owner, repo, attempt + 1).await
                }
                _ => Err(e),
            }
        }
    }
}

async fn get_timeline(
    number: u64,
    owner: String,
    repo: String,
) -> Result<Vec<models::timelines::TimelineEvent>, octocrab::Error> {
    let mut events = Vec::<models::timelines::TimelineEvent>::new();

    for page in 1..u32::MAX {
        match get_timeline_page(number, page, owner.clone(), repo.clone(), 0).await {
            Ok(mut events_page) => {
                events.append(&mut events_page.take_items());

                debug!(
                    "loaded {} events for issue {} in {}:{}",
                    events.len(),
                    number,
                    owner,
                    repo
                );

                if events_page.next.is_none() {
                    return Ok(events);
                }
            }
            Err(e) => return Err(e),
        }
    }

    Ok(events)
}

#[async_recursion]
async fn get_issue_page(
    page: u32,
    since: Option<DateTime<Utc>>,
    owner: String,
    repo: String,
    attempt: u8,
) -> octocrab::Result<Page<octocrab::models::issues::Issue>> {
    let mut sort = params::issues::Sort::Created;
    // if we have a since DateTime, sort by when the Issue was last updated
    if since.is_some() {
        sort = params::issues::Sort::Updated;
    }

    match octocrab::instance()
        .issues(&owner, &repo)
        .list()
        .per_page(100)
        .direction(params::Direction::Ascending)
        .sort(sort)
        .since(since.unwrap_or_default())
        .state(params::State::All)
        .page(page)
        .send()
        .await
    {
        Ok(p) => Ok(p),
        Err(e) => {
            match e {
                octocrab::Error::GitHub { .. } => {
                    if attempt > 0 {
                        return Err(e);
                    }
                    // retry once incase we hit the rate-limiting
                    wait_on_ratelimit().await;
                    get_issue_page(page, since, owner, repo, attempt + 1).await
                }
                _ => Err(e),
            }
        }
    }
}

async fn get_pull(
    number: u64,
    owner: String,
    repo: String,
) -> Result<EntryWithMetadata, octocrab::Error> {
    let body_future = get_pull_body(number, owner.clone(), repo.clone(), 0);
    let events_future = get_timeline(number, owner.clone(), repo.clone());
    let comments_future = get_pull_comments(number, owner, repo);

    let pull = match body_future.await {
        Ok(pull) => pull,
        Err(e) => {
            error!("Error in get_pull_body() for pull={}: {}", number, e);
            return Err(e);
        }
    };
    let events = match events_future.await {
        Ok(events) => events,
        Err(e) => {
            error!("Error in get_timeline() for pull={}: {}", number, e);
            return Err(e);
        }
    };
    let comments = match comments_future.await {
        Ok(events) => events,
        Err(e) => {
            error!("Error in get_pull_comments() for pull={}: {}", number, e);
            return Err(e);
        }
    };

    Ok(EntryWithMetadata::Pull(PullWithMetadata::new(
        pull, events, comments,
    )))
}

async fn get_issue(
    issue: issues::Issue,
    number: u64,
    owner: String,
    repo: String,
) -> Result<EntryWithMetadata, octocrab::Error> {
    let events_future = get_timeline(number, owner.clone(), repo.clone());

    let events = match events_future.await {
        Ok(events) => events,
        Err(e) => {
            error!("Error in get_timeline() for issue={}: {}", number, e);
            return Err(e);
        }
    };

    Ok(EntryWithMetadata::Issue(IssueWithMetadata::new(
        issue, events,
    )))
}

async fn get_issues_and_pulls(
    sender: mpsc::Sender<EntryWithMetadata>,
    since: Option<DateTime<Utc>>,
    owner: String,
    repo: String,
) -> Result<(), octocrab::Error> {
    let mut loaded_issues: usize = 0;
    let mut loaded_pulls: usize = 0;
    let mut failed_issues: Vec<u64> = Vec::new();
    let mut failed_pulls: Vec<u64> = Vec::new();
    info!(
        "Start to load issues and pulls for {}:{} from GitHub",
        owner, repo
    );
    for page_num in START_PAGE..u32::MAX {
        let page = match get_issue_page(page_num, since, owner.clone(), repo.clone(), 0).await {
            Ok(page) => page,
            Err(e) => {
                error!(
                    "Could not load issue page {} for {}:{} from GitHub: {}",
                    page_num, owner, repo, e
                );
                return Err(e);
            }
        };

        for entry in page.items {
            if entry.pull_request.is_none() {
                match get_issue(entry.clone(), entry.number, owner.clone(), repo.clone()).await {
                    Ok(issue) => {
                        sender.send(issue).await.unwrap();
                        loaded_issues += 1;
                    }
                    Err(e) => {
                        error!("Could not get issue #{}: {}", entry.number, e);
                        failed_issues.push(entry.number);
                    }
                }
            } else {
                match get_pull(entry.number, owner.clone(), repo.clone()).await {
                    Ok(pull) => {
                        sender.send(pull).await.unwrap();
                        loaded_pulls += 1;
                    }
                    Err(e) => {
                        error!("Could not get pull-request #{}: {}", entry.number, e);
                        failed_pulls.push(entry.number);
                    }
                }
            }
        }

        if page.next.is_none() {
            break;
        }
    }
    info!(
        "Loaded {} issues and {} pulls from {}:{}",
        loaded_issues, loaded_pulls, owner, repo
    );
    if !failed_issues.is_empty() {
        warn!(
            "The following {} issues failed to load: {:?}",
            failed_issues.len(),
            failed_issues
        );
    }
    if !failed_issues.is_empty() {
        warn!(
            "The following {} pulls failed to load: {:?}",
            failed_pulls.len(),
            failed_pulls
        );
    }

    Ok(())
}

fn write(x: EntryWithMetadata, destination: PathBuf) -> Result<(), WriteError> {
    let mut path = destination;
    let json: String = match x {
        EntryWithMetadata::Issue(i) => {
            path.push("issues");
            path.push(format!("{}.json", i.issue.number));
            serde_json::to_string_pretty(&i)?
        }
        EntryWithMetadata::Pull(p) => {
            path.push("pulls");
            path.push(format!("{}.json", p.pull.number));
            serde_json::to_string_pretty(&p)?
        }
    };
    let mut file = File::create(path.clone())?;
    file.write_all(json.as_bytes())?;
    info!("Written {}", path.display());
    Ok(())
}

fn write_backup_state(start_time: DateTime<Utc>) {
    let state = BackupState {
        version: STATE_VERSION,
        last_backup: start_time,
    };
    let json = serde_json::to_string_pretty(&state).unwrap();
    let mut file = File::create(STATE_FILE_PATH).unwrap();
    file.write_all(json.as_bytes()).unwrap();
    info!("written {}", STATE_FILE_PATH);
}

fn get_last_backup_time(destination: PathBuf) -> Option<DateTime<Utc>> {
    let mut path = destination;
    path.push(STATE_FILE_PATH);
    info!("Trying to read {} file", path.display());
    match fs::read_to_string(path.clone()) {
        Ok(contents) => {
            info!("Trying deserialize {} file", path.display());
            match serde_json::from_str::<BackupState>(&contents) {
                Ok(state) => match state.version {
                    STATE_VERSION => {
                        info!(
                            "Doing an incremental GitHub backup starting from {}.",
                            state.last_backup
                        );
                        Some(state.last_backup)
                    }
                    _ => {
                        warn!("BackupState version {} is unknown.", state.version);
                        None
                    }
                },
                Err(e) => {
                    warn!(
                        "BackupState file {} could not be deserialized: {}",
                        path.display(),
                        e
                    );
                    None
                }
            }
        }
        Err(e) => {
            info!(
                "BackupState file {} could not be found: {}",
                path.display(),
                e
            );
            None
        }
    }
}

fn personal_access_token(args: Args) -> Option<String> {
    if let Some(pat) = args.personal_access_token {
        info!("Using the GitHub personal access token specified on the command line");
        return Some(pat);
    } else if let Some(pat_file) = args.personal_access_token_file {
        info!(
            "Reading the GitHub personal access token from '{}'",
            pat_file.display()
        );
        match fs::read_to_string(pat_file.clone()) {
            Ok(pat) => {
                return Some(pat.trim().to_string());
            }
            Err(e) => {
                error!(
                    "Could not read GitHub personal access token from '{}': {}",
                    pat_file.display(),
                    e
                );
                return None;
            }
        }
    }
    None
}

#[tokio::main]
async fn main() -> ExitCode {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let args: Args = Args::parse();
    info!(
        "Starting backup of {}:{} on GitHub to '{}'",
        args.owner,
        args.repo,
        args.destination.display()
    );

    let pat = match personal_access_token(args.clone()) {
        Some(pat) => pat,
        None => {
            error!("No GitHub personal access token present - exiting.");
            return ExitCode::from(EXIT_NO_PAT);
        }
    };

    let issues_dir = args.destination.join("issues");
    let pulls_dir = args.destination.join("pulls");
    info!(
        "If not existing yet, creating 'issues' and 'pulls' directory as {} and {}",
        issues_dir.display(),
        pulls_dir.display()
    );
    if let Err(e) = fs::create_dir_all(issues_dir.clone()) {
        error!(
            "Could not create 'issues' directory in {}: {}",
            issues_dir.display(),
            e
        );
        return ExitCode::from(EXIT_CREATING_DIRS);
    }
    if let Err(e) = fs::create_dir_all(pulls_dir.clone()) {
        error!(
            "Could not create 'pulls' directory in {}: {}",
            pulls_dir.display(),
            e
        );
        return ExitCode::from(EXIT_CREATING_DIRS);
    }

    let start_time = chrono::Utc::now();
    let last_backup_time: Option<DateTime<Utc>> = get_last_backup_time(args.destination.clone());

    let instance = match octocrab::OctocrabBuilder::default()
        .personal_token(pat)
        .build()
    {
        Ok(instance) => instance,
        Err(e) => {
            error!(
                "Could not create Octocrab instance with the supplied personal access token: {}",
                e
            );
            return ExitCode::from(EXIT_CREATING_OCTOCRAB_INSTANCE);
        }
    };
    octocrab::initialise(instance);

    // Fetched issues and PRs are send into this mpsc channel and received by
    // the writer which persist them to the disk.
    let (sender, mut receiver) = mpsc::channel(100);

    let task = task::spawn(async move {
        if let Err(e) = get_issues_and_pulls(sender, last_backup_time, args.owner, args.repo).await
        {
            panic!("Error loading issues and pulls: {}", e);
        }
    });

    while let Some(data) = receiver.recv().await {
        if let Err(e) = write(data.clone(), args.destination.clone()) {
            error!(
                "Could not write {} to {}: {}",
                data,
                args.destination.clone().display(),
                e
            );
            receiver.close();
            return ExitCode::from(EXIT_WRITING);
        }
    }

    if task.await.is_ok() {
        write_backup_state(start_time);
    } else {
        return ExitCode::from(EXIT_API_ERROR);
    }

    ExitCode::SUCCESS
}
