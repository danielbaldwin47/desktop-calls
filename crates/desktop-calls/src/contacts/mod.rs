//! Contacts sync: reads the Phone's Contacts and Call History over PBAP,
//! through obexd, and keeps a copy on disk.
//!
//! It takes commands one at a time from a queue, so there is never more
//! than one PBAP session to the Phone. What it hides:
//!
//! - **The Phone's prompt.** The first session asks the person holding the
//!   Phone whether to share Contacts, for up to 30 s, but obexd gives up
//!   after 10 s. While `CreateSession` is pending the sync reports that
//!   permission is needed, and it asks again until the Phone's deadline.
//! - **What it pulls.** `pb` for Contacts, and `cch` for the Call History,
//!   or, where the Phone lacks `cch`, `ich`, `och` and `mch` merged without
//!   duplicates. A folder the Phone can't give doesn't stop the others.
//! - **Pages.** The Pixel gives at most 50 Call History entries a pull, so
//!   each folder is read with `Offset` until `GetSize` is reached. A sync
//!   reports Contacts and the newest page first, then the rest; after a
//!   Call, only the newest page is pulled again, in front of the older
//!   Calls held.
//! - **vCards** (`vcard.rs`), asked for as 3.0, read as 3.0 or 2.1, and
//!   Call History times without a time zone read as the Desktop's local time.
//! - **The cache** (`cache.rs`), read when the Phone is attached, written
//!   after every pull, and deleted when the Phone refuses.
//!
//! Reading files, parsing and indexing run on a thread of their own, so the
//! main loop, and so the Banner, never waits for them.

mod cache;
mod obex;
mod vcard;

use std::collections::HashSet;
use std::fs::{self, DirBuilder};
use std::os::unix::fs::DirBuilderExt as _;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use desktop_calls_core::{
    CallHistory, CallHistoryEntry, CallHistoryKind, Caller, Contact, ContactNumber, Contacts,
    Directory, Input, SyncFailure, Timestamp, Unavailable,
};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures_channel::oneshot;
use futures_util::StreamExt as _;
use futures_util::future::{Either, select};
use zbus::{Connection, OwnedObjectPath};

use crate::logging::EventKind;
use cache::Cached;
use obex::ObexError;

/// How long `CreateSession` may be pending before the Phone is taken to be
/// asking whether to share Contacts.
const PROMPT_SUSPECTED_AFTER: Duration = Duration::from_secs(2);
/// How long Android shows that prompt.
const PROMPT_TIMEOUT: Duration = Duration::from_secs(30);
/// The most pages read from one folder: 400 of the Pixel's 50-entry
/// Call History pages is 20,000 Calls.
const MOST_PAGES: usize = 400;

/// What the Contacts sync can be told to do.
pub enum Command {
    /// Work with the Phone at this Bluetooth address, starting from its
    /// cache.
    Attach { address: String },
    /// Stop working with any Phone.
    Detach,
    /// Sync Contacts and Call History.
    Sync,
    /// Pull the Call History again.
    RefreshCallHistory,
    /// Delete everything held for the Phone, on disk too.
    Forget,
}

/// What the sync holds for the attached Phone, to write to the cache.
#[derive(Default)]
struct Held {
    address: Option<String>,
    cached: Cached,
}

/// Runs the Contacts sync until `commands` closes, reporting to `inputs`.
pub async fn run(
    session: Connection,
    mut commands: UnboundedReceiver<Command>,
    inputs: UnboundedSender<Input>,
) {
    let sync = ContactsSync {
        session,
        inputs,
        locale_region: locale_region(),
    };
    let mut held = Held::default();
    while let Some(command) = commands.next().await {
        match command {
            Command::Attach { address } => {
                held = Held {
                    address: Some(address.clone()),
                    cached: Cached::default(),
                };
                if let Some(cached) = sync.load(&address).await {
                    held.cached = cached;
                }
            }
            Command::Detach => held = Held::default(),
            Command::Sync => {
                if let Some(address) = held.address.clone() {
                    sync.sync(&address, &mut held.cached).await;
                }
            }
            Command::RefreshCallHistory => {
                // Every refresh asked for ends in a report, so the Call state
                // machine can tell which refresh confirms which Missed Call.
                if let Some(address) = held.address.clone() {
                    sync.refresh_call_history(&address, &mut held.cached).await;
                } else {
                    sync.report(Input::CallHistoryRefreshFailed);
                }
            }
            Command::Forget => {
                held.cached = Cached::default();
                if let Some(address) = held.address.clone() {
                    off_main_thread(move || cache::forget(&address))
                        .await
                        .unwrap_or(Ok(()))
                        .unwrap_or_else(|error| cache_failed(&error));
                }
            }
        }
    }
}

struct ContactsSync {
    session: Connection,
    inputs: UnboundedSender<Input>,
    locale_region: Option<String>,
}

/// Why a sync gave up.
#[derive(Clone, Copy)]
enum Stop {
    Denied,
    Unavailable(Unavailable),
}

impl ContactsSync {
    fn report(&self, input: Input) {
        let _ = self.inputs.unbounded_send(input);
    }

    /// Reads the cache, and reports what is in it.
    async fn load(&self, address: &str) -> Option<Cached> {
        let address = address.to_owned();
        let region = self.locale_region.clone();
        let loaded = off_main_thread(move || {
            let cached = cache::load(&address)?;
            Ok::<_, cache::CacheError>(cached.map(|cached| {
                let indexed = index(&cached, region.as_deref());
                (cached, indexed)
            }))
        })
        .await?;
        match loaded {
            Ok(Some((cached, (contacts, call_history)))) => {
                tracing::info!(
                    event_kind = %EventKind::ContactsLoaded,
                    contacts = cached.contacts.len(),
                    call_history = cached.call_history.len(),
                    "Read {} Contacts and {} Call History entries from the cache",
                    cached.contacts.len(),
                    cached.call_history.len()
                );
                self.report(Input::ContactsLoaded {
                    contacts,
                    call_history,
                });
                Some(cached)
            }
            Ok(None) => None,
            Err(error) => {
                cache_failed(&error);
                None
            }
        }
    }

    /// Pulls Contacts and Call History in one session.
    async fn sync(&self, address: &str, held: &mut Cached) {
        tracing::info!(event_kind = %EventKind::ContactsSyncStarted, "Syncing Contacts");
        let session = match self.open_session(address).await {
            Ok(session) => session,
            Err(stop) => return self.stopped(stop),
        };
        let pulled = self.sync_in(&session, address, held).await;
        self.close_session(&session).await;
        if let Err(stop) = pulled {
            self.stopped(stop);
        }
    }

    /// Pulls Contacts and the newest page of the Call History, and reports
    /// them; then reads the rest of the Call History, page by page, and
    /// reports it too. So the Banner can name callers, and Missed Calls and
    /// the Dialer show recent Calls, without waiting for years of Calls.
    async fn sync_in(
        &self,
        session: &OwnedObjectPath,
        address: &str,
        held: &mut Cached,
    ) -> Result<(), Stop> {
        let mut contacts = Folder::new("pb", None);
        let has_contacts = self.pull(session, &mut contacts, MOST_PAGES).await?;
        let history = self.pull_call_history(session, 1).await?;
        if !has_contacts && history.is_none() {
            return Err(Stop::Unavailable(Unavailable::Failed));
        }
        // A folder that couldn't be pulled keeps what was held.
        if has_contacts {
            held.contacts = contacts_from(contacts.cards);
        }
        if let Some(folders) = &history {
            held.call_history = call_history_from(folders, std::mem::take(&mut held.call_history));
        }
        let (contacts, call_history) = self.index_and_save(address, held).await;
        tracing::info!(
            event_kind = %EventKind::ContactsSynced,
            contacts = held.contacts.len(),
            call_history = held.call_history.len(),
            "Synced {} Contacts and {} Call History entries",
            held.contacts.len(),
            held.call_history.len()
        );
        self.report(Input::ContactsSynced {
            contacts,
            call_history,
        });

        let Some(mut folders) = history else {
            return Ok(());
        };
        if folders.iter().all(|folder| folder.complete) {
            return Ok(());
        }
        for folder in folders.iter_mut().filter(|folder| !folder.complete) {
            self.pull(session, folder, MOST_PAGES).await?;
        }
        held.call_history = call_history_from(&folders, std::mem::take(&mut held.call_history));
        let (contacts, call_history) = self.index_and_save(address, held).await;
        tracing::info!(
            event_kind = %EventKind::ContactsSynced,
            call_history = held.call_history.len(),
            "Read the rest of the Call History: {} entries",
            held.call_history.len()
        );
        // Part of the sync, not a refresh: a refresh confirms Missed Calls.
        self.report(Input::ContactsSynced {
            contacts,
            call_history,
        });
        Ok(())
    }

    /// Pulls the newest page of the Call History in a session of its own,
    /// and puts it in front of the older Calls held. Reports the Call
    /// History, or that the refresh failed.
    async fn refresh_call_history(&self, address: &str, held: &mut Cached) {
        let session = match self.open_session(address).await {
            Ok(session) => session,
            Err(stop) => return self.refresh_stopped(Some(stop)),
        };
        let pulled = self.pull_call_history(&session, 1).await;
        self.close_session(&session).await;
        match pulled {
            Err(stop) => self.refresh_stopped(Some(stop)),
            Ok(None) => self.refresh_stopped(None),
            Ok(Some(folders)) => {
                held.call_history =
                    call_history_from(&folders, std::mem::take(&mut held.call_history));
                let (_, call_history) = self.index_and_save(address, held).await;
                tracing::info!(
                    event_kind = %EventKind::CallHistoryRefreshed,
                    call_history = held.call_history.len(),
                    "Refreshed the Call History: {} entries",
                    held.call_history.len()
                );
                self.report(Input::CallHistoryRefreshed(call_history));
            }
        }
    }

    /// A refresh ended without a Call History: because the sync had to stop,
    /// or because the Phone offers no Call History folder.
    fn refresh_stopped(&self, stop: Option<Stop>) {
        if let Some(stop) = stop {
            self.stopped(stop);
        }
        tracing::info!(
            event_kind = %EventKind::CallHistoryRefreshFailed,
            "Couldn't refresh the Call History"
        );
        self.report(Input::CallHistoryRefreshFailed);
    }

    fn stopped(&self, stop: Stop) {
        let failure = match stop {
            Stop::Denied => SyncFailure::Denied,
            Stop::Unavailable(reason) => SyncFailure::Unavailable(reason),
        };
        self.report(Input::ContactsSyncFailed(failure));
    }

    /// Opens a PBAP session, waiting out the Phone's prompt: while
    /// `CreateSession` is pending, permission is needed, and each time obexd
    /// gives up first, it asks again, until the prompt's deadline.
    async fn open_session(&self, address: &str) -> Result<OwnedObjectPath, Stop> {
        let started = Instant::now();
        let deadline = now() + millis(PROMPT_TIMEOUT);
        let mut prompt_reported = false;
        loop {
            let attempt = Box::pin(obex::create_session(&self.session, address));
            let result = if prompt_reported {
                attempt.await
            } else {
                match select(
                    attempt,
                    Box::pin(glib::timeout_future(PROMPT_SUSPECTED_AFTER)),
                )
                .await
                {
                    Either::Left((result, _)) => result,
                    Either::Right(((), attempt)) => {
                        prompt_reported = true;
                        tracing::info!(
                            event_kind = %EventKind::ContactsPermissionNeeded,
                            "The Phone is asking whether to share Contacts"
                        );
                        self.report(Input::ContactsPermissionNeeded {
                            deadline: Timestamp(deadline),
                        });
                        attempt.await
                    }
                }
            };
            let error = match result {
                Ok(session) => return Ok(session),
                Err(error) => error,
            };
            sync_failed("CreateSession", &error);
            match error {
                ObexError::TimedOut
                    if started.elapsed() + Duration::from_secs(1) < PROMPT_TIMEOUT =>
                {
                    if !prompt_reported {
                        prompt_reported = true;
                        self.report(Input::ContactsPermissionNeeded {
                            deadline: Timestamp(deadline),
                        });
                    }
                }
                ObexError::TimedOut if prompt_reported => {
                    return Err(Stop::Unavailable(Unavailable::PromptTimedOut));
                }
                other => return Err(stop_for(&other)),
            }
        }
    }

    async fn close_session(&self, session: &OwnedObjectPath) {
        if let Err(error) = obex::remove_session(&self.session, session).await {
            sync_failed("RemoveSession", &error);
        }
    }

    /// Pulls up to `pages` pages of `cch`, or else of `mch`, `ich` and
    /// `och`; `None` if the Phone can give none of them.
    async fn pull_call_history(
        &self,
        session: &OwnedObjectPath,
        pages: usize,
    ) -> Result<Option<Vec<Folder>>, Stop> {
        let mut combined = Folder::new("cch", None);
        if self.pull(session, &mut combined, pages).await? {
            return Ok(Some(vec![combined]));
        }
        let mut folders = Vec::new();
        // Missed first: a Phone that also lists a Missed Call as incoming
        // keeps it as missed.
        for (name, kind) in [
            ("mch", CallHistoryKind::Missed),
            ("ich", CallHistoryKind::Incoming),
            ("och", CallHistoryKind::Outgoing),
        ] {
            let mut folder = Folder::new(name, Some(kind));
            if self.pull(session, &mut folder, pages).await? {
                folders.push(folder);
            }
        }
        Ok((!folders.is_empty()).then_some(folders))
    }

    /// Reads up to `pages` more pages of `folder`, until it is complete.
    /// False if the Phone can't give the folder at all. Stops the sync only
    /// if the Phone refuses, or obexd is gone.
    async fn pull(
        &self,
        session: &OwnedObjectPath,
        folder: &mut Folder,
        pages: usize,
    ) -> Result<bool, Stop> {
        if let Err(error) = obex::select_folder(&self.session, session, folder.name).await {
            return pull_failed(folder.name, &error).map(|()| false);
        }
        if folder.size.is_none() {
            folder.size = obex::size(&self.session, session)
                .await
                .inspect_err(|error| sync_failed("GetSize", error))
                .ok();
        }
        for _ in 0..pages {
            if folder.complete {
                break;
            }
            let Ok(offset) = u16::try_from(folder.cards.len()) else {
                folder.complete = true;
                break;
            };
            let page = match self.pull_page(session, folder.name, offset).await {
                Ok(Some(page)) => page,
                Ok(None) if offset == 0 => return Ok(false),
                Ok(None) => {
                    folder.complete = true;
                    break;
                }
                Err(error) => {
                    pull_failed(folder.name, &error)?;
                    if offset == 0 {
                        return Ok(false);
                    }
                    // Keeps the pages already read.
                    folder.complete = true;
                    break;
                }
            };
            // A Phone that ignores `Offset` gives the first page again.
            if page.is_empty() || (offset > 0 && folder.cards.starts_with(&page)) {
                folder.complete = true;
                break;
            }
            folder.cards.extend(page);
            if folder
                .size
                .is_some_and(|size| folder.cards.len() >= usize::from(size))
            {
                folder.complete = true;
            }
        }
        Ok(true)
    }

    /// Pulls the selected folder's vCards from the `offset`th on, and reads
    /// them; `None` if obexd said they were pulled but the file can't be
    /// read.
    async fn pull_page(
        &self,
        session: &OwnedObjectPath,
        folder: &str,
        offset: u16,
    ) -> Result<Option<Vec<vcard::Card>>, ObexError> {
        let Some(target) = pull_target(folder) else {
            return Err(ObexError::Other(
                "no runtime directory to pull into".to_owned(),
            ));
        };
        let pulled = obex::pull(&self.session, session, offset, &target).await;
        let read = off_main_thread(move || {
            let text = fs::read(&target);
            let _ = fs::remove_file(&target);
            text.map(|text| vcard::parse(&text))
        })
        .await;
        // Whatever obexd wrote before it failed is deleted all the same.
        pulled?;
        if let Some(Ok(cards)) = read {
            tracing::debug!(
                event_kind = %EventKind::ContactsPulled,
                folder,
                offset,
                vcards = cards.len(),
                "Pulled {} vCards from {folder} at {offset}",
                cards.len()
            );
            Ok(Some(cards))
        } else {
            tracing::warn!(
                event_kind = %EventKind::ContactsSyncFailed,
                error_kind = "pulled-file-unreadable",
                folder,
                "obexd said {folder} was pulled, but its file can't be read"
            );
            Ok(None)
        }
    }

    /// Indexes what is held, off the main thread, and writes the cache.
    async fn index_and_save(&self, address: &str, held: &mut Cached) -> (Contacts, CallHistory) {
        let address = address.to_owned();
        let region = self.locale_region.clone();
        let taken = std::mem::take(held);
        let done = off_main_thread(move || {
            let indexed = index(&taken, region.as_deref());
            let saved = cache::save(&address, &taken);
            (taken, indexed, saved)
        })
        .await;
        let Some((taken, indexed, saved)) = done else {
            return (Contacts::default(), CallHistory::default());
        };
        *held = taken;
        if let Err(error) = saved {
            cache_failed(&error);
        }
        indexed
    }
}

/// What the sync does when `error` stops it.
fn stop_for(error: &ObexError) -> Stop {
    match error {
        ObexError::Forbidden => Stop::Denied,
        ObexError::NoService => Stop::Unavailable(Unavailable::NoContactsService),
        _ => Stop::Unavailable(Unavailable::Failed),
    }
}

/// Logs a folder that couldn't be read; stops the sync only if the Phone
/// refuses, or obexd is gone.
fn pull_failed(folder: &str, error: &ObexError) -> Result<(), Stop> {
    sync_failed(folder, error);
    match error {
        ObexError::Forbidden | ObexError::NoService => Err(stop_for(error)),
        _ => Ok(()),
    }
}

/// One of the Phone's folders, as read so far, a page at a time.
struct Folder {
    name: &'static str,
    /// What kind of Call each entry is, where its vCard doesn't say.
    kind: Option<CallHistoryKind>,
    cards: Vec<vcard::Card>,
    /// How many vCards the Phone says the folder holds, once asked.
    size: Option<u16>,
    /// Whether every page has been read, or no more can be.
    complete: bool,
}

impl Folder {
    fn new(name: &'static str, kind: Option<CallHistoryKind>) -> Self {
        Self {
            name,
            kind,
            cards: Vec::new(),
            size: None,
            complete: false,
        }
    }
}

/// Indexes Contacts for matching, and the Call History for the core.
fn index(cached: &Cached, locale_region: Option<&str>) -> (Contacts, CallHistory) {
    let directory = Directory::new(cached.contacts.clone(), locale_region);
    let call_history = CallHistory::new(cached.call_history.clone(), &directory);
    (Contacts::new(directory), call_history)
}

fn contacts_from(cards: Vec<vcard::Card>) -> Vec<Contact> {
    cards
        .into_iter()
        .filter(|card| !card.name.trim().is_empty() || !card.numbers.is_empty())
        .map(|card| Contact {
            name: card.name,
            numbers: card
                .numbers
                .into_iter()
                .map(|(number, label)| ContactNumber { number, label })
                .collect(),
        })
        .collect()
}

/// Call History entries from each folder's vCards, without duplicates: the
/// same number at the same time is one Call. A folder's `kind` is used
/// where the vCard doesn't say.
fn merge(folders: &[Folder]) -> Vec<CallHistoryEntry> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for folder in folders {
        for card in &folder.cards {
            let stamp = card.call.as_ref();
            let Some(kind) = stamp.and_then(|stamp| stamp.kind).or(folder.kind) else {
                continue;
            };
            let at = stamp.and_then(|stamp| stamp.at).and_then(timestamp);
            let caller = card
                .numbers
                .first()
                .map_or(Caller::Withheld, |(number, _)| {
                    Caller::Number(number.clone())
                });
            let entry = CallHistoryEntry {
                kind,
                caller,
                at,
                name: card.name.clone(),
            };
            if is_new(&mut seen, &entry) {
                entries.push(entry);
            }
        }
    }
    entries
}

/// Whether `entry` is a Call not yet in `seen`, which it is then added to:
/// the same number at the same time is one Call. Withheld Calls with no
/// time can't be told apart, so each is new.
fn is_new(seen: &mut HashSet<(String, Option<u64>)>, entry: &CallHistoryEntry) -> bool {
    let digits: String = match &entry.caller {
        Caller::Number(number) => number.chars().filter(char::is_ascii_digit).collect(),
        Caller::Withheld => String::new(),
    };
    (entry.at.is_none() && digits.is_empty()) || seen.insert((digits, entry.at.map(|at| at.0)))
}

/// The Call History from what was pulled of `folders`, and, where a folder
/// wasn't read to the end, the `held` Calls older than any it gave.
fn call_history_from(folders: &[Folder], held: Vec<CallHistoryEntry>) -> Vec<CallHistoryEntry> {
    let mut entries = merge(folders);
    // Every folder was read back to this moment; before it, `held` knows.
    let read_back_to = folders
        .iter()
        .filter(|folder| !folder.complete)
        .map(|folder| {
            merge(std::slice::from_ref(folder))
                .iter()
                .filter_map(|entry| entry.at)
                .min()
        })
        .max();
    let Some(Some(read_back_to)) = read_back_to else {
        return entries;
    };
    let mut seen = HashSet::new();
    for entry in &entries {
        is_new(&mut seen, entry);
    }
    entries.extend(
        held.into_iter().filter(|entry| {
            entry.at.is_some_and(|at| at < read_back_to) && is_new(&mut seen, entry)
        }),
    );
    entries
}

/// The moment `when` names: in the offset it gives, or else in the
/// Desktop's local time zone (`TZ`, or `/etc/localtime`).
fn timestamp(when: vcard::DateTime) -> Option<Timestamp> {
    let zone = match when.utc_offset {
        Some(offset) => glib::TimeZone::from_offset(offset),
        None => glib::TimeZone::local(),
    };
    let moment = glib::DateTime::new(
        &zone,
        when.year,
        when.month,
        when.day,
        when.hour,
        when.minute,
        f64::from(when.second),
    )
    .ok()?;
    let millis = moment.to_unix().checked_mul(1_000)?;
    u64::try_from(millis).ok().map(Timestamp)
}

/// Where obexd writes a pulled folder: a private directory for this run,
/// in `$XDG_RUNTIME_DIR`, or else the cache directory.
fn pull_target(folder: &str) -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|runtime| runtime.join("desktop-calls"));
    let directory = match runtime {
        Some(directory) => {
            DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&directory)
                .ok()?;
            directory
        }
        None => cache::private_directory().ok()?,
    };
    Some(directory.join(format!("pull-{folder}.vcf")))
}

/// The region of the Desktop's locale for telephone numbers, such as `GB`
/// for `en_GB.UTF-8`.
fn locale_region() -> Option<String> {
    ["LC_ALL", "LC_TELEPHONE", "LANG"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
        .and_then(|locale| {
            let (_, rest) = locale.split_once('_')?;
            let region: String = rest.chars().take_while(char::is_ascii_alphabetic).collect();
            (region.len() == 2).then_some(region)
        })
}

/// Runs `work` on a thread of its own and waits for it without blocking
/// the main loop. `None` if the thread couldn't start or panicked.
async fn off_main_thread<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (sender, receiver) = oneshot::channel();
    std::thread::Builder::new()
        .name("contacts".to_owned())
        .spawn(move || {
            let _ = sender.send(work());
        })
        .ok()?;
    receiver.await.ok()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn sync_failed(step: &str, error: &ObexError) {
    tracing::warn!(
        event_kind = %EventKind::ContactsSyncFailed,
        error_kind = error.as_str(),
        step,
        detail = error.detail(),
        "Contacts sync: {step} failed"
    );
}

fn cache_failed(error: &cache::CacheError) {
    let detail = match error {
        cache::CacheError::Io(error) => error.to_string(),
        _ => String::new(),
    };
    tracing::warn!(
        event_kind = %EventKind::ContactsCacheFailed,
        error_kind = error.as_str(),
        detail,
        "Couldn't use the Contacts cache"
    );
}
