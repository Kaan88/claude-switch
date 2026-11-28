//! Asynchronous session reader. The selected conversation uses stable item IDs
//! until `generation` changes. Agents and tasks always describe the root.
mod file_refs;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::SystemTime;

use claude_switch_core::transcript::{
    Changes, Item, ItemId, ItemKind, NoticeKind, Picture, Reader, Subagent, Task, ToolState,
};
use mdshared_text::image::Image;
use mdshared_ui::thumbs::MemoryCache;

use super::links::{self, Files, NamedFile, ShellLogs};
use super::{
    documents::{self, Document},
    outputs::{self, Output},
};
use claude_switch_core::transcript::{BackgroundShell, McpTask, ShellStatus};

const CHUNK: usize = 256 * 1024;

#[derive(Clone)]
pub enum Source {
    File(PathBuf),
    Demo(PathBuf),
    Discover { config: PathBuf, cwd: PathBuf, started: SystemTime, verified: Arc<std::sync::OnceLock<String>> },
}

/// What a session ID's transcript looks like on disk for a [`Source::Discover`] pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Probe {
    /// The file exists, is new enough and records the tab's folder.
    Found(PathBuf),
    /// No file with that name yet: Claude Code writes it with the session's first prompt.
    Missing,
    /// Anything else: an invalid ID, an old or foreign file, or a read error.
    Rejected,
}

impl Source {
    /// A pane that follows `id` for the same process: an existing transcript of any age (`/resume`)
    /// or one the CLI has yet to write (`/clear`). `None` when this source has no project folder.
    pub(super) fn follow(&self, cwd: &Path, id: &str) -> Option<Self> {
        let config = match self {
            Self::Discover { config, .. } => config.clone(),
            Self::File(path) => {
                let dir = path.parent()?;
                let config = dir.parent()?.parent()?.to_owned();
                (project(&config, cwd) == dir).then_some(config)?
            }
            Self::Demo(_) => return None,
        };
        let verified = Arc::new(std::sync::OnceLock::new());
        verified.set(id.to_owned()).ok()?;
        Some(Self::Discover { config, cwd: cwd.to_owned(), started: SystemTime::UNIX_EPOCH, verified })
    }

    pub(super) fn probe(&self, id: &str) -> Probe {
        match self {
            Self::Discover { config, cwd, started, .. } => probe(&project(config, cwd), cwd, *started, id),
            Self::File(path) | Self::Demo(path) => Probe::Found(path.clone()),
        }
    }

    /// Pins `id` as this pane's session when nothing is pinned yet. False when another is.
    pub(super) fn pin(&self, id: &str) -> bool {
        match self {
            Self::Discover { verified, .. } => verified.get_or_init(|| id.to_owned()).eq_ignore_ascii_case(id),
            Self::File(_) | Self::Demo(_) => true,
        }
    }

    pub fn find_file(&self) -> Option<PathBuf> {
        match self {
            Self::File(path) | Self::Demo(path) => Some(path.clone()),
            Self::Discover { config, cwd, started, verified } => {
                discover(&project(config, cwd), cwd, *started, verified.get()?).ok().flatten()
            }
        }
    }
}

pub(super) fn project(config: &Path, cwd: &Path) -> PathBuf {
    config.join("projects").join(project_slug(cwd))
}

pub(crate) fn project_slug(cwd: &Path) -> String {
    let cwd = cwd.to_string_lossy();
    cwd.trim_start_matches(r"\\?\").chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

fn discover(dir: &Path, cwd: &Path, started: SystemTime, verified: &str) -> io::Result<Option<PathBuf>> {
    if claude_switch_core::launch::validate_session_id(verified).is_err() {
        return Ok(None);
    }
    let path = dir.join(format!("{verified}.jsonl"));
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    Ok((meta.is_file() && meta.created()? >= started && super::discovery::cwd_matches(&path, cwd)).then_some(path))
}

fn probe(dir: &Path, cwd: &Path, started: SystemTime, id: &str) -> Probe {
    if claude_switch_core::launch::validate_session_id(id).is_err() {
        return Probe::Rejected;
    }
    match discover(dir, cwd, started, id) {
        Ok(Some(path)) => Probe::Found(path),
        Ok(None) if matches!(fs::symlink_metadata(dir.join(format!("{id}.jsonl"))), Err(e) if e.kind() == io::ErrorKind::NotFound) => {
            Probe::Missing
        }
        _ => Probe::Rejected,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Group {
    One(ItemId),
    Tools(Vec<ItemId>),
}

pub(super) fn group(items: &BTreeMap<ItemId, Item>) -> Vec<Group> {
    let mut groups = Vec::new();
    for (&id, item) in items {
        // A thought between tool calls stays in their group instead of splitting it.
        if matches!(item.kind, ItemKind::Tool { .. } | ItemKind::Notice { kind: NoticeKind::Thought, .. }) {
            if let Some(Group::Tools(ids)) = groups.last_mut() {
                ids.push(id);
            } else {
                groups.push(Group::Tools(vec![id]));
            }
        } else {
            groups.push(Group::One(id));
        }
    }
    groups
}

#[derive(Clone, Copy, Debug)]
pub struct Follow(bool);
impl Default for Follow {
    fn default() -> Self {
        Self(true)
    }
}
impl Follow {
    pub fn scrolled(&mut self, offset: f32, max: f32) {
        self.0 = offset >= max - 1.0;
    }
    pub fn latest(&mut self) {
        self.0 = true;
    }
    pub fn pause(&mut self) {
        self.0 = false;
    }
    pub fn active(&self) -> bool {
        self.0
    }
}

#[derive(Clone, Copy, Hash, Eq, PartialEq)]
struct Key {
    offset: u64,
    block: usize,
    index: usize,
    large: bool,
}
impl Key {
    fn new(id: ItemId, index: usize, large: bool) -> Self {
        Self { offset: id.offset, block: id.block, index, large }
    }
}

enum Command {
    Poll,
    Earlier(u64),
    Select(Option<String>),
    Picture(u64, Key, Picture),
    Open(u64, Picture),
    Markdown(u64, u64, NamedFile),
    ShellTail(u64, ItemId),
}

#[derive(Default)]
struct Update {
    reset: bool,
    items: Vec<(ItemId, Item)>,
    agents: Option<Vec<Subagent>>,
    tasks: Option<Vec<Task>>,
    earlier: bool,
    loaded_earlier: bool,
    agent: Option<String>,
    session: Option<String>,
    generation: u64,
    chat: claude_switch_core::transcript::ChatInfo,
    files: Vec<(ItemId, Files)>,
    outputs: Vec<(ItemId, Vec<Output>)>,
    shells: Option<Vec<BackgroundShell>>,
    mcp_tasks: Option<Vec<McpTask>>,
}

enum Event {
    Update(Box<Update>),
    Picture(u64, Key, io::Result<Arc<Image>>),
    Markdown(u64, u64, io::Result<Document>),
    ShellTail(u64, ItemId, Option<Vec<String>>),
    Error(Option<String>),
}

pub struct Model {
    pub items: BTreeMap<ItemId, Item>,
    pub groups: Vec<Group>,
    pub agents: Vec<Subagent>,
    pub tasks: Vec<Task>,
    pub agent: Option<String>,
    pub has_earlier: bool,
    pub earlier_version: u64,
    pub error: Option<String>,
    /// The reader has delivered the transcript at least once (it may hold nothing).
    pub loaded: bool,
    pub revision: u64,
    pub generation: u64,
    pub chat: claude_switch_core::transcript::ChatInfo,
    pub files: BTreeMap<ItemId, Files>,
    pub(super) outputs: BTreeMap<ItemId, Vec<Output>>,
    pub(super) output_list: Vec<Output>,
    pub(super) shells: Vec<BackgroundShell>,
    pub(super) mcp_tasks: Vec<McpTask>,
    shell_logs: ShellLogs,
    pub(super) shell_tails: BTreeMap<ItemId, Option<Vec<String>>>,
    tail_requested: BTreeSet<ItemId>,
    markdown_ticket: u64,
    markdown: Option<io::Result<Document>>,
    session: Option<String>,
    tx: mpsc::Sender<Command>,
    rx: mpsc::Receiver<Event>,
    polling: Arc<AtomicBool>,
    cache: MemoryCache<Key>,
    requested: HashSet<Key>,
    /// Pictures that could not be shown, with the reason said under them.
    failed: HashMap<Key, &'static str>,
}

impl Model {
    pub fn new(source: Source, cwd: PathBuf, wake: impl Fn() + Send + 'static) -> Self {
        Self::after(source, cwd, 0, wake)
    }

    /// A reader whose generations continue after `generation`, so state keyed to an earlier
    /// reader's generation (selection, shells, recall) cannot match this one.
    pub fn after(source: Source, cwd: PathBuf, generation: u64, wake: impl Fn() + Send + 'static) -> Self {
        let temp = if matches!(source, Source::Demo(_)) { cwd.clone() } else { std::env::temp_dir() };
        let shell_logs = ShellLogs::new(&temp, &cwd);
        let worker_logs = shell_logs.clone();
        let (tx, commands) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        let polling = Arc::new(AtomicBool::new(false));
        let queued = polling.clone();
        std::thread::spawn(move || {
            let mut worker = Worker {
                source,
                cwd,
                root: None,
                child: None,
                session: None,
                generation,
                chat: Default::default(),
                refs: Default::default(),
                shells: Default::default(),
                mcp_tasks: Default::default(),
                shell_logs: worker_logs,
            };
            let mut error = None;
            for command in commands {
                if matches!(command, Command::Poll) {
                    queued.store(false, Ordering::Release);
                }
                let result = worker.run(command);
                let new_error = result.as_ref().err().map(ToString::to_string);
                let mut changed = false;
                if let Ok(Some(event)) = result {
                    if events.send(event).is_err() {
                        break;
                    }
                    changed = true;
                }
                if error != new_error {
                    error = new_error;
                    if events.send(Event::Error(error.clone())).is_err() {
                        break;
                    }
                    changed = true;
                }
                if changed {
                    wake();
                }
            }
        });
        Self {
            items: Default::default(),
            groups: vec![],
            agents: vec![],
            tasks: vec![],
            agent: None,
            has_earlier: false,
            earlier_version: 0,
            error: None,
            loaded: false,
            revision: 0,
            generation,
            chat: Default::default(),
            files: Default::default(),
            outputs: Default::default(),
            output_list: Vec::new(),
            shells: Vec::new(),
            mcp_tasks: Vec::new(),
            shell_logs,
            shell_tails: Default::default(),
            tail_requested: Default::default(),
            markdown_ticket: 0,
            markdown: None,
            session: None,
            tx,
            rx,
            polling,
            cache: MemoryCache::new(48 * 1024 * 1024),
            requested: Default::default(),
            failed: Default::default(),
        }
    }
    pub fn poll(&self) {
        if !self.polling.swap(true, Ordering::AcqRel) {
            let _ = self.tx.send(Command::Poll);
        }
    }
    pub fn collect(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Update(u) => {
                    self.loaded = true;
                    if u.reset {
                        self.items.clear();
                        self.files.clear();
                        self.outputs.clear();
                        self.shell_tails.clear();
                        self.tail_requested.clear();
                        self.cancel_markdown();
                        self.cache.remove_where(|_| true);
                        self.requested.clear();
                        self.failed.clear();
                    }
                    if u.reset || !u.outputs.is_empty() {
                        for (id, outputs) in u.outputs {
                            if outputs.is_empty() {
                                self.outputs.remove(&id);
                            } else {
                                self.outputs.insert(id, outputs);
                            }
                        }
                        self.output_list = outputs::latest(&self.outputs);
                    }
                    if let Some(tasks) = u.mcp_tasks {
                        self.mcp_tasks = tasks;
                    }
                    if let Some(shells) = u.shells {
                        self.shells = shells;
                        self.shell_tails.retain(|id, _| self.shells.iter().any(|s| s.tool == *id));
                    }
                    for (id, files) in u.files {
                        if files.is_empty() {
                            self.files.remove(&id);
                        } else {
                            self.files.insert(id, files);
                        }
                    }
                    let regroup = u.reset || !u.items.is_empty();
                    for (id, item) in u.items {
                        let changed_pictures = self.items.get(&id).is_some_and(|old| refs(old) != refs(&item));
                        self.items.insert(id, item);
                        if changed_pictures {
                            self.cache.remove_where(|k| k.offset == id.offset && k.block == id.block);
                            self.requested.retain(|k| k.offset != id.offset || k.block != id.block);
                            self.failed.retain(|k, _| k.offset != id.offset || k.block != id.block);
                        }
                    }
                    if regroup {
                        self.groups = group(&self.items);
                    }
                    if let Some(agents) = u.agents {
                        self.agents = agents;
                    }
                    if let Some(tasks) = u.tasks {
                        self.tasks = tasks;
                    }
                    self.has_earlier = u.earlier;
                    if u.loaded_earlier {
                        self.earlier_version = self.earlier_version.wrapping_add(1);
                    }
                    self.generation = u.generation;
                    self.agent = u.agent;
                    self.session = u.session;
                    self.chat = u.chat;
                    self.revision += 1;
                }
                Event::Picture(g, key, result) if g == self.generation => {
                    self.requested.remove(&key);
                    match result {
                        Ok(image) => self.cache.insert(key, image),
                        Err(error) => {
                            self.failed.insert(key, super::pictures::reason(&error));
                        }
                    }
                }
                Event::Picture(..) => continue,
                Event::Markdown(g, ticket, result) if g == self.generation && ticket == self.markdown_ticket => {
                    self.markdown = Some(result);
                }
                Event::Markdown(..) => continue,
                Event::ShellTail(g, id, lines) if g == self.generation => {
                    self.tail_requested.remove(&id);
                    if self.shell_tails.get(&id) == Some(&lines) {
                        continue;
                    }
                    self.shell_tails.insert(id, lines);
                }
                Event::ShellTail(..) => continue,
                Event::Error(e) => {
                    self.error = e;
                    self.revision += 1;
                }
            }
            changed = true;
        }
        changed
    }
    pub fn session_id(&self) -> Option<&str> {
        self.session.as_deref()
    }

    /// Where a branch at the message `item` cuts the transcript ([`branch_cut`]); `None` in a
    /// subagent's transcript, whose items are not the session's.
    pub fn branch_cut(&self, item: ItemId) -> Option<claude_switch_core::branch::Cut> {
        if self.agent.is_some() {
            return None;
        }
        branch_cut(&self.items, &self.chat, item)
    }
    pub fn earlier(&self) {
        let _ = self.tx.send(Command::Earlier(self.generation));
    }
    pub fn select_agent(&self, agent: Option<&str>) {
        let _ = self.tx.send(Command::Select(agent.map(str::to_owned)));
    }
    /// Every picture of the conversation shown, in conversation order: a prompt's pasted
    /// pictures and a tool's results, each as its item and its place there. Nothing is loaded.
    pub fn pictures(&self) -> impl Iterator<Item = (ItemId, usize)> + '_ {
        self.items.iter().flat_map(|(&id, item)| (0..refs(item).len()).map(move |index| (id, index)))
    }
    /// Whether a picture's preview was asked of the worker, loaded or not.
    #[cfg(test)]
    pub fn picture_asked(&mut self, id: ItemId, index: usize, large: bool) -> bool {
        let key = Key::new(id, index, large);
        self.requested.contains(&key) || self.cache.get(&key).is_some() || self.failed.contains_key(&key)
    }
    fn reference(&self, id: ItemId, index: usize) -> Option<&Picture> {
        refs(self.items.get(&id)?).get(index)
    }
    /// Why a picture cannot be shown, once its decode failed.
    pub fn picture_failed(&self, id: ItemId, index: usize, large: bool) -> Option<&'static str> {
        self.failed.get(&Key::new(id, index, large)).copied()
    }
    pub fn picture(&mut self, id: ItemId, index: usize, large: bool) -> Option<Arc<Image>> {
        let key = Key::new(id, index, large);
        if let Some(image) = self.cache.get(&key) {
            return Some(image);
        }
        if !self.requested.contains(&key) && !self.failed.contains_key(&key) {
            let pic = self.reference(id, index)?.clone();
            self.requested.insert(key);
            let _ = self.tx.send(Command::Picture(self.generation, key, pic));
        }
        None
    }
    pub fn open_picture(&self, id: ItemId, index: usize) {
        if let Some(pic) = self.reference(id, index) {
            let _ = self.tx.send(Command::Open(self.generation, pic.clone()));
        }
    }

    pub fn request_markdown(&mut self, file: NamedFile) {
        self.cancel_markdown();
        if self.tx.send(Command::Markdown(self.generation, self.markdown_ticket, file)).is_err() {
            self.markdown = Some(Err(io::Error::other("The transcript worker is no longer available.")));
        }
    }

    pub(super) fn take_markdown(&mut self) -> Option<io::Result<Document>> {
        self.markdown.take()
    }

    pub(super) fn running_shells(&self) -> usize {
        self.shells.iter().take_while(|s| s.status == ShellStatus::Running).count()
    }

    pub(super) fn shell_log(&self, shell: &BackgroundShell) -> Option<NamedFile> {
        self.shell_logs.named(&shell.id, &shell.output, self.session_id())
    }

    pub(super) fn request_shell_tail(&mut self, id: ItemId) {
        if self.tail_requested.insert(id) && self.tx.send(Command::ShellTail(self.generation, id)).is_err() {
            self.tail_requested.remove(&id);
        }
    }

    fn cancel_markdown(&mut self) {
        self.markdown_ticket = self.markdown_ticket.wrapping_add(1);
        self.markdown = None;
    }
}

struct Worker {
    source: Source,
    cwd: PathBuf,
    root: Option<Reader>,
    child: Option<(String, Reader)>,
    session: Option<String>,
    generation: u64,
    chat: claude_switch_core::transcript::ChatInfo,
    refs: file_refs::FileRefs,
    shells: Vec<BackgroundShell>,
    mcp_tasks: Vec<McpTask>,
    shell_logs: ShellLogs,
}

fn refs(item: &Item) -> &[Picture] {
    match &item.kind {
        ItemKind::Prompt { pictures, .. } | ItemKind::Tool { pictures, .. } => pictures,
        _ => &[],
    }
}

fn named_paths(item: &Item) -> Vec<String> {
    match &item.kind {
        ItemKind::Assistant { markdown } => super::preview::markdown_names(markdown),
        ItemKind::Prompt { text, .. } | ItemKind::Notice { text, .. } => {
            links::detect(text).into_iter().map(|(_, name)| name).collect()
        }
        ItemKind::Tool { file_path: Some(path), .. } => vec![path.clone()],
        _ => Vec::new(),
    }
}

impl Worker {
    fn run(&mut self, command: Command) -> io::Result<Option<Event>> {
        let mut reset = false;
        let mut lists = Changes::default();
        let mut visible = Changes::default();
        let mut earlier_request = false;
        if self.root.is_none() {
            if !matches!(command, Command::Poll) {
                return Ok(None);
            }
            let path = match &self.source {
                Source::File(path) | Source::Demo(path) => path.clone(),
                Source::Discover { config, cwd, started, verified } => {
                    let id = verified.get().ok_or_else(|| {
                        io::Error::other("Transcript not attached: session identity is unverified or ambiguous.")
                    })?;
                    let Some(path) = discover(&project(config, cwd), cwd, *started, id)? else { return Ok(None) };
                    path
                }
            };
            self.root = Some(Reader::open(&path, CHUNK)?);
            self.session = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned);
            reset = true;
            lists.reset = true;
        }
        let root = self.root.as_mut().expect("reader opened");
        match command {
            Command::Poll if !reset => {
                lists = root.poll()?;
                if lists.reset
                    || self
                        .child
                        .as_ref()
                        .is_some_and(|(id, _)| lists.subagents_changed && !root.subagents().any(|a| &a.id == id))
                {
                    self.child = None;
                    reset = true;
                }
                if let Some((_, child)) = &mut self.child {
                    visible = child.poll()?;
                }
            }
            Command::Poll => {}
            Command::Select(id) => {
                if id.as_deref() == self.child.as_ref().map(|(id, _)| id.as_str()) {
                    return Ok(None);
                }
                self.child = match id {
                    Some(id) => {
                        let child = root.open_subagent(&id, CHUNK)?;
                        Some((id, child))
                    }
                    None => None,
                };
                reset = true;
            }
            Command::Earlier(g) if g == self.generation => {
                let reader = self.child.as_mut().map(|(_, r)| r).unwrap_or(root);
                if !reader.has_earlier() {
                    return Ok(None);
                }
                earlier_request = true;
                let c = reader.earlier()?;
                if self.child.is_some() {
                    visible = c;
                } else {
                    lists = c;
                }
            }
            Command::Picture(g, key, pic) if g == self.generation => {
                let reader = self.child.as_mut().map(|(_, r)| r).unwrap_or(root);
                return Ok(Some(Event::Picture(g, key, super::pictures::preview(reader, &pic, key.large))));
            }
            Command::Open(g, pic) if g == self.generation => {
                let reader = self.child.as_mut().map(|(_, r)| r).unwrap_or(root);
                super::pictures::open(reader, &pic)?;
                return Ok(None);
            }
            Command::Markdown(g, ticket, file) if g == self.generation => {
                return Ok(Some(Event::Markdown(g, ticket, documents::read(&file))));
            }
            Command::ShellTail(g, id) if g == self.generation => {
                let lines = self
                    .shells
                    .iter()
                    .find(|shell| shell.tool == id && shell.status == ShellStatus::Running)
                    .and_then(|shell| self.shell_logs.named(&shell.id, &shell.output, self.session.as_deref()))
                    .and_then(|file| documents::tail(&file).ok());
                return Ok(Some(Event::ShellTail(g, id, lines)));
            }
            _ => return Ok(None),
        }
        let reader = self.child.as_ref().map(|(_, r)| r).unwrap_or(root);
        let change = if self.child.is_some() { &visible } else { &lists };
        reset |= change.reset;
        if reset {
            self.generation += 1;
        }
        // A prompt after a rewind leaves the replaced turns off the branch: the items are shown
        // afresh, but the reader is the same, so the generation (history pages, pictures, recall)
        // stays.
        let rebuild = reset || change.branch_changed;
        let mut u = Update {
            reset: rebuild,
            earlier: reader.has_earlier(),
            loaded_earlier: earlier_request,
            agent: self.child.as_ref().map(|(id, _)| id.clone()),
            session: self.session.clone(),
            generation: self.generation,
            chat: root.chat(),
            ..Update::default()
        };
        let metadata_changed = self.chat != u.chat;
        self.chat.clone_from(&u.chat);
        if lists.reset || lists.tasks_changed {
            u.tasks = Some(root.tasks().to_vec());
        }
        if lists.reset || lists.subagents_changed {
            u.agents = Some(root.subagents().cloned().collect());
        }
        if rebuild {
            u.items = reader.items().map(|(id, item)| (id, item.clone())).collect();
        } else {
            u.items =
                change.added.union(&change.updated).filter_map(|&id| Some((id, reader.item(id)?.clone()))).collect();
        }
        u.files = self.refs.update(rebuild, &u.items, &self.cwd, |id| match reader.item(id).map(|item| &item.kind) {
            Some(ItemKind::Tool { state: claude_switch_core::transcript::ToolState::Ok, .. }) => reader
                .result_details(id)
                .map(|result| result.saved_paths().map(str::to_owned).collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        });
        let ids = u.items.iter().map(|(id, _)| *id).chain(u.files.iter().map(|(id, _)| *id)).collect::<BTreeSet<_>>();
        u.outputs = ids
            .into_iter()
            .filter_map(|id| {
                Some((id, outputs::collect(id, reader.item(id)?, reader.result_details(id), self.refs.get(id))))
            })
            .collect();
        if rebuild
            || change.background_changed
            || u.items.iter().any(|(_, item)| matches!(&item.kind, ItemKind::Tool { .. }))
        {
            let shells = reader.background_shells();
            if rebuild || self.shells != shells {
                self.shells.clone_from(&shells);
                u.shells = Some(shells);
            }
            let tasks = reader.mcp_tasks();
            if rebuild || self.mcp_tasks != tasks {
                self.mcp_tasks.clone_from(&tasks);
                u.mcp_tasks = Some(tasks);
            }
        }
        Ok((rebuild
            || earlier_request
            || metadata_changed
            || !u.items.is_empty()
            || !u.files.is_empty()
            || u.shells.is_some()
            || u.mcp_tasks.is_some()
            || u.tasks.is_some()
            || u.agents.is_some())
        .then(|| Event::Update(Box::new(u))))
    }
}

/// Where a branch at the message `item` cuts the transcript: at the end of that message's turn,
/// once the turn has ended, so that the copy ends where Claude Code can go on. A turn ends before
/// the next prompt typed at an idle input (a message sent while it ran goes on in it), and the
/// newest one at the record of its end (`end_turn`, no call pending, no prompt after it). The
/// message is a prompt of the user's or its turn's last answer. `None` for anything else, for a
/// turn still running, and when the copy would hold a tool call without its result (Claude Code
/// cannot go on from one).
pub(crate) fn branch_cut(
    items: &BTreeMap<ItemId, Item>,
    chat: &claude_switch_core::transcript::ChatInfo,
    item: ItemId,
) -> Option<claude_switch_core::branch::Cut> {
    use claude_switch_core::branch::Cut;
    use claude_switch_core::transcript::ChatPhase;
    use std::ops::Bound::{Excluded, Unbounded};
    let answer = match &items.get(&item)?.kind {
        ItemKind::Prompt { synthetic: false, .. } => false,
        ItemKind::Assistant { .. } => true,
        _ => return None,
    };
    // Every call the copy holds has its result.
    let whole = |end: ItemId| {
        !items.range(..end).any(|(_, i)| matches!(i.kind, ItemKind::Tool { state: ToolState::Running, .. }))
    };
    let mut answered = false;
    for (id, next) in items.range((Excluded(item), Unbounded)) {
        match next.kind {
            ItemKind::Prompt { synthetic: false, queued: false, .. } => {
                return (!(answer && answered) && whole(*id)).then_some(Cut::Before(id.offset));
            }
            ItemKind::Assistant { .. } => answered = true,
            _ => {}
        }
    }
    if answer && answered {
        return None;
    }
    let (at, phase) = chat.latest?;
    let prompt_after = items
        .range(ItemId { offset: at.saturating_add(1), block: 0 }..)
        .any(|(_, i)| matches!(i.kind, ItemKind::Prompt { synthetic: false, .. }));
    let ended = phase == ChatPhase::EndTurn && chat.pending.is_none() && !prompt_after && at >= item.offset;
    let end = ItemId { offset: at.saturating_add(1), block: 0 };
    (ended && whole(end)).then_some(Cut::After(at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use claude_switch_core::transcript::ToolState;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn adjacent_tools_group_including_agents_until_a_message_breaks_the_run() {
        let tool = |agent| ItemKind::Tool {
            id: String::new(),
            name: String::new(),
            summary: String::new(),
            file_path: None,
            state: ToolState::Ok,
            duration_ms: None,
            pictures: vec![],
            subagent: agent,
            card: None,
            denial: None,
        };
        let id = |offset| ItemId { offset, block: 0 };
        let items = [
            tool(None),
            tool(None),
            tool(Some("scout".into())),
            tool(None),
            ItemKind::Assistant { markdown: "Done".into() },
            tool(None),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, kind)| (id(i as u64), Item { kind, timestamp: None }))
        .collect();
        assert_eq!(
            group(&items),
            [Group::Tools(vec![id(0), id(1), id(2), id(3)]), Group::One(id(4)), Group::Tools(vec![id(5)]),]
        );
    }

    #[test]
    fn scrolling_away_pauses_follow_until_latest_or_scroll_to_bottom() {
        let mut follow = Follow::default();
        assert!(follow.active());
        follow.scrolled(40.0, 68.0);
        assert!(!follow.active());
        follow.scrolled(120.0, 120.5);
        assert!(follow.active());
        follow.scrolled(0.0, 120.5);
        follow.latest();
        assert!(follow.active());
    }

    #[test]
    fn discovery_uses_only_new_top_level_transcripts_in_the_encoded_folder() {
        let scratch = crate::testing::Scratch::new("claude-switch-discovery");
        let config = scratch.to_path_buf();
        let cwd = Path::new(r"C:\Work\rail yard");
        let dir = project(&config, cwd);
        assert_eq!(dir, config.join("projects").join("C--Work-rail-yard"));
        let id = "01234567-89ab-cdef-0123-456789abcdef";
        assert_eq!(discover(&dir, cwd, UNIX_EPOCH, id).unwrap(), None);
        fs::create_dir_all(dir.join("subagents")).unwrap();
        fs::write(dir.join("subagents/scout.jsonl"), b"{}\n").unwrap();
        fs::write(dir.join("note.txt"), b"").unwrap();
        let file = dir.join(format!("{id}.jsonl"));
        fs::write(&file, b"{}\n").unwrap();
        fs::write(dir.join("fedcba98-7654-3210-fedc-ba9876543210.jsonl"), b"{}\n").unwrap();
        let created = fs::metadata(&file).unwrap().created().unwrap();
        assert_eq!(discover(&dir, cwd, created, id).unwrap().as_ref(), Some(&file));
        assert_eq!(discover(&dir, cwd, created + Duration::from_secs(1), id).unwrap(), None);
        assert_eq!(discover(&dir, cwd, UNIX_EPOCH, "11234567-89ab-cdef-0123-456789abcdef").unwrap(), None);
        fs::write(&file, b"{\"cwd\":\"C:/Work/rail.yard\"}\n").unwrap();
        assert_eq!(discover(&dir, cwd, UNIX_EPOCH, id).unwrap(), None);
        fs::remove_dir_all(config).unwrap();
    }

    #[test]
    fn a_rewinds_next_prompt_drops_the_replaced_turns_and_keeps_the_generation() {
        let dir = crate::testing::unique("cs-model-rewind");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.jsonl");
        let line = |v: serde_json::Value| format!("{v}\n");
        let prompt = |uuid: &str, parent: Option<&str>, text: &str| {
            line(serde_json::json!({"type":"user","uuid":uuid,"parentUuid":parent,"message":{"content":text}}))
        };
        let answer = |uuid: &str, parent: &str, text: &str| {
            line(serde_json::json!({"type":"assistant","uuid":uuid,"parentUuid":parent,
                "message":{"content":[{"type":"text","text":text}],"stop_reason":"end_turn"}}))
        };
        let two = prompt("u1", None, "one")
            + &answer("a1", "u1", "first")
            + &prompt("u2", Some("a1"), "two")
            + &answer("a2", "u2", "second");
        fs::write(&path, two).unwrap();
        let (wake, events) = std::sync::mpsc::channel();
        let mut model = Model::new(Source::File(path.clone()), dir.clone(), move || {
            let _ = wake.send(());
        });
        let texts = |model: &Model| -> Vec<String> {
            model
                .items
                .values()
                .filter_map(|item| match &item.kind {
                    ItemKind::Prompt { text, .. } => Some(format!("> {text}")),
                    ItemKind::Assistant { markdown } => Some(markdown.clone()),
                    _ => None,
                })
                .collect()
        };
        let settle = |model: &mut Model, want: usize| {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while std::time::Instant::now() < deadline {
                model.poll();
                let _ = events.recv_timeout(Duration::from_millis(50));
                model.collect();
                if texts(model).len() == want {
                    break;
                }
            }
        };
        settle(&mut model, 4);
        assert_eq!(texts(&model), ["> one", "first", "> two", "second"]);
        let generation = model.generation;
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        std::io::Write::write_all(&mut file, prompt("u3", Some("a1"), "two again").as_bytes()).unwrap();
        settle(&mut model, 3);
        assert_eq!(texts(&model), ["> one", "first", "> two again"]);
        assert_eq!(model.generation, generation, "the same reader: pages, pictures and recall go on");
        drop(model);
        let _ = fs::remove_dir_all(&dir);
    }
}
