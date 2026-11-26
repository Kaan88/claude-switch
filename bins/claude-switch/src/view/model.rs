//! Asynchronous session reader. The selected conversation uses stable item IDs
//! until `generation` changes. Agents and tasks always describe the root.
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::SystemTime;

use claude_switch_core::transcript::{Changes, Item, ItemId, ItemKind, Picture, Reader, Subagent, Task};
use mdshared_text::image::Image;
use mdshared_ui::thumbs::MemoryCache;

const CHUNK: usize = 256 * 1024;

pub enum Source {
    File(PathBuf),
    Discover { config: PathBuf, cwd: PathBuf, started: SystemTime },
}

fn project(config: &Path, cwd: &Path) -> PathBuf {
    let cwd = cwd.to_string_lossy();
    let encoded: String =
        cwd.trim_start_matches(r"\\?\").chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    config.join("projects").join(encoded)
}

fn discover(dir: &Path, cwd: &Path, started: SystemTime) -> io::Result<Option<PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        other => other?,
    };
    Ok(entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()? != "jsonl" || !entry.file_type().ok()?.is_file() {
                return None;
            }
            let created = entry.metadata().ok()?.created().ok()?;
            (created >= started && super::discovery::cwd_matches(&path, cwd)).then_some((created, path))
        })
        .max()
        .map(|(_, path)| path))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Group {
    One(ItemId),
    Tools(Vec<ItemId>),
}

fn group(items: &BTreeMap<ItemId, Item>) -> Vec<Group> {
    let mut groups = Vec::new();
    for (&id, item) in items {
        if matches!(item.kind, ItemKind::Tool { .. }) {
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
}

#[derive(Default)]
struct Update {
    reset: bool,
    items: Vec<(ItemId, Item)>,
    agents: Option<Vec<Subagent>>,
    tasks: Option<Vec<Task>>,
    earlier: bool,
    agent: Option<String>,
    session: Option<String>,
    generation: u64,
}

enum Event {
    Update(Update),
    Picture(u64, Key, io::Result<Arc<Image>>),
    Error(Option<String>),
}

pub struct Model {
    pub items: BTreeMap<ItemId, Item>,
    pub groups: Vec<Group>,
    pub agents: Vec<Subagent>,
    pub tasks: Vec<Task>,
    pub agent: Option<String>,
    pub has_earlier: bool,
    pub error: Option<String>,
    pub revision: u64,
    pub generation: u64,
    session: Option<String>,
    tx: mpsc::Sender<Command>,
    rx: mpsc::Receiver<Event>,
    polling: Arc<AtomicBool>,
    cache: MemoryCache<Key>,
    requested: HashSet<Key>,
    failed: HashSet<Key>,
}

impl Model {
    pub fn new(source: Source, wake: impl Fn() + Send + 'static) -> Self {
        let (tx, commands) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        let polling = Arc::new(AtomicBool::new(false));
        let queued = polling.clone();
        std::thread::spawn(move || {
            let mut worker = Worker { source, root: None, child: None, session: None, generation: 0 };
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
            error: None,
            revision: 0,
            generation: 0,
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
                    if u.reset {
                        self.items.clear();
                        self.cache.remove_where(|_| true);
                        self.requested.clear();
                        self.failed.clear();
                    }
                    let regroup = u.reset || !u.items.is_empty();
                    for (id, item) in u.items {
                        let changed_pictures = self.items.get(&id).is_some_and(|old| refs(old) != refs(&item));
                        self.items.insert(id, item);
                        if changed_pictures {
                            self.cache.remove_where(|k| k.offset == id.offset && k.block == id.block);
                            self.requested.retain(|k| k.offset != id.offset || k.block != id.block);
                            self.failed.retain(|k| k.offset != id.offset || k.block != id.block);
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
                    self.generation = u.generation;
                    self.agent = u.agent;
                    self.session = u.session;
                    self.revision += 1;
                }
                Event::Picture(g, key, result) if g == self.generation => {
                    self.requested.remove(&key);
                    match result {
                        Ok(image) => self.cache.insert(key, image),
                        Err(_) => {
                            self.failed.insert(key);
                        }
                    }
                }
                Event::Picture(..) => continue,
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
    pub fn earlier(&self) {
        let _ = self.tx.send(Command::Earlier(self.generation));
    }
    pub fn select_agent(&self, agent: Option<&str>) {
        let _ = self.tx.send(Command::Select(agent.map(str::to_owned)));
    }
    fn reference(&self, id: ItemId, index: usize) -> Option<&Picture> {
        refs(self.items.get(&id)?).get(index)
    }
    pub fn picture_failed(&self, id: ItemId, index: usize, large: bool) -> bool {
        self.failed.contains(&Key::new(id, index, large))
    }
    pub fn picture(&mut self, id: ItemId, index: usize, large: bool) -> Option<Arc<Image>> {
        let key = Key::new(id, index, large);
        if let Some(image) = self.cache.get(&key) {
            return Some(image);
        }
        if !self.requested.contains(&key) && !self.failed.contains(&key) {
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
}

struct Worker {
    source: Source,
    root: Option<Reader>,
    child: Option<(String, Reader)>,
    session: Option<String>,
    generation: u64,
}

fn refs(item: &Item) -> &[Picture] {
    match &item.kind {
        ItemKind::Prompt { pictures, .. } | ItemKind::Tool { pictures, .. } => pictures,
        _ => &[],
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
                Source::File(path) => path.clone(),
                Source::Discover { config, cwd, started } => {
                    let Some(path) = discover(&project(config, cwd), cwd, *started)? else { return Ok(None) };
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
            _ => return Ok(None),
        }
        let reader = self.child.as_ref().map(|(_, r)| r).unwrap_or(root);
        let change = if self.child.is_some() { &visible } else { &lists };
        reset |= change.reset;
        if reset {
            self.generation += 1;
        }
        let mut u = Update {
            reset,
            earlier: reader.has_earlier(),
            agent: self.child.as_ref().map(|(id, _)| id.clone()),
            session: self.session.clone(),
            generation: self.generation,
            ..Update::default()
        };
        if lists.reset || lists.tasks_changed {
            u.tasks = Some(root.tasks().to_vec());
        }
        if lists.reset || lists.subagents_changed {
            u.agents = Some(root.subagents().cloned().collect());
        }
        if reset {
            u.items = reader.items().map(|(id, item)| (id, item.clone())).collect();
        } else {
            u.items =
                change.added.union(&change.updated).filter_map(|&id| Some((id, reader.item(id)?.clone()))).collect();
        }
        Ok((reset || earlier_request || !u.items.is_empty() || u.tasks.is_some() || u.agents.is_some())
            .then_some(Event::Update(u)))
    }
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
            state: ToolState::Ok,
            pictures: vec![],
            subagent: agent,
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
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let config = std::env::temp_dir().join(format!("claude-switch-discovery-{}-{nonce}", std::process::id()));
        let cwd = Path::new(r"C:\Work\rail yard");
        let dir = project(&config, cwd);
        assert_eq!(dir, config.join("projects").join("C--Work-rail-yard"));
        assert_eq!(discover(&dir, cwd, UNIX_EPOCH).unwrap(), None);
        fs::create_dir_all(dir.join("subagents")).unwrap();
        fs::write(dir.join("subagents/scout.jsonl"), b"{}\n").unwrap();
        fs::write(dir.join("note.txt"), b"").unwrap();
        let file = dir.join("session.jsonl");
        fs::write(&file, b"{}\n").unwrap();
        let created = fs::metadata(&file).unwrap().created().unwrap();
        assert_eq!(discover(&dir, cwd, created).unwrap().as_ref(), Some(&file));
        assert_eq!(discover(&dir, cwd, created + Duration::from_secs(1)).unwrap(), None);
        fs::write(&file, b"{\"cwd\":\"C:/Work/rail.yard\"}\n").unwrap();
        assert_eq!(discover(&dir, cwd, UNIX_EPOCH).unwrap(), None);
        fs::remove_dir_all(config).unwrap();
    }
}
