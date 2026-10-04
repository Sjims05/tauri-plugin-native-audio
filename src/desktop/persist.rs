//! What the desktop player saves between runs, in the app's data folder (native-audio/): settings,
//! the last queue, the playback event log, tracked lists, item progress and the progress
//! checkpoint. The same data and rules as on Android (PluginSettings, QueueSnapshotStore,
//! PlaybackEvents, TrackedLists, ItemProgress), as JSON files.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::player::Item;

const SETTINGS: &str = "settings.json";
const QUEUE: &str = "queue.json";
const QUEUE_POSITION: &str = "queue-position.json";
const PLAYBACK_EVENTS: &str = "playback-events.json";
const TRACKED_LISTS: &str = "tracked-lists.json";
const ITEM_PROGRESS: &str = "item-progress.json";
const CHECKPOINT: &str = "progress-checkpoint.json";

const MAX_PLAYBACK_EVENTS: usize = 500;
const DEFAULT_LIST_LIMIT: usize = 10;
/// From here on an item counts as played (e.g. skipped credits at the end of an episode).
const PLAYED_AT: f64 = 0.98;

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// A random-enough unique id for log entries.
fn new_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    format!("{:x}-{:x}-{:x}", nanos, std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// Settings the app set, so they're the same next time without setting them again.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub volume: Option<f32>,
    pub volume_curve: Option<String>,
    pub output_device: Option<String>,
    pub skip_interval: Option<f64>,
    pub repeat_added_tracks: Option<bool>,
    pub media_controls: Option<bool>,
    pub track_progress: Option<bool>,
    /// "off", "paused" or "play" (true / false from before "play" existed: paused / off).
    pub resume_last_queue: Option<Value>,
    pub previous_restarts_after_seconds: Option<f64>,
    pub next_at_end: Option<String>,
}

/// The queue as saved: everything but where playback is (QueuePosition, saved far more often).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedQueue {
    pub items: Vec<SavedItem>,
    /// List indices in play order.
    pub order: Vec<usize>,
    pub shuffle: bool,
    pub repeat_mode: String,
    #[serde(default)]
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedItem {
    #[serde(flatten)]
    pub item: Item,
    #[serde(default)]
    pub added: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuePosition {
    pub index: usize,
    pub position_secs: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListConfig {
    pub id: String,
    /// "folder": the playable folder a queue was started from (setQueue's sourceId). "item": tracks.
    pub track: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub count_after_seconds: f64,
}

fn default_limit() -> usize {
    DEFAULT_LIST_LIMIT
}

/// A tracked list entry: `id` is a folder id (string) or an item id (number).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListEntry {
    pub id: Value,
    pub played_at_ms: i64,
}

impl ListEntry {
    fn key(&self) -> String {
        match &self.id {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct TrackedListsFile {
    configs: Vec<ListConfig>,
    lists: HashMap<String, Vec<ListEntry>>,
    /// The latest unacknowledged change per list.
    changes: HashMap<String, Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub progress: f64,
    pub position_ms: i64,
    pub updated_at_ms: i64,
}

/// Everything saved, loaded once and written through.
pub struct Persist {
    dir: Option<PathBuf>,
    settings: Settings,
    tracked: TrackedListsFile,
    progress: HashMap<i64, Progress>,
    events: Vec<Value>,
}

impl Persist {
    /// `dir`: where to save, or None to keep everything in memory only.
    pub fn open(dir: Option<PathBuf>) -> Self {
        if let Some(dir) = &dir {
            if let Err(e) = fs::create_dir_all(dir) {
                eprintln!("native-audio: can't create {}: {e}", dir.display());
            }
        }
        let mut persist = Self { dir, settings: Settings::default(), tracked: Default::default(), progress: HashMap::new(), events: Vec::new() };
        persist.settings = persist.read(SETTINGS).unwrap_or_default();
        persist.tracked = persist.read(TRACKED_LISTS).unwrap_or_default();
        let progress: HashMap<String, Progress> = persist.read(ITEM_PROGRESS).unwrap_or_default();
        persist.progress = progress.into_iter().filter_map(|(k, v)| Some((k.parse().ok()?, v))).collect();
        persist.events = persist.read(PLAYBACK_EVENTS).unwrap_or_default();
        persist
    }

    fn read<T: DeserializeOwned>(&self, name: &str) -> Option<T> {
        let path = self.dir.as_ref()?.join(name);
        let text = fs::read_to_string(&path).ok()?;
        match serde_json::from_str(&text) {
            Ok(value) => Some(value),
            Err(e) => {
                eprintln!("native-audio: ignoring unreadable {}: {e}", path.display());
                None
            }
        }
    }

    /// Writes to a temporary file first, so a crash mid-write never leaves a broken file.
    fn write<T: Serialize>(&self, name: &str, value: &T) {
        let Some(dir) = &self.dir else { return };
        let result = serde_json::to_vec(value).map_err(|e| e.to_string()).and_then(|bytes| {
            let tmp = dir.join(format!("{name}.tmp"));
            fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
            fs::rename(&tmp, dir.join(name)).map_err(|e| e.to_string())
        });
        if let Err(e) = result {
            eprintln!("native-audio: can't save {name}: {e}");
        }
    }

    fn remove(&self, name: &str) {
        if let Some(dir) = &self.dir {
            let _ = fs::remove_file(dir.join(name));
        }
    }

    // ---- settings

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn update_settings(&mut self, change: impl FnOnce(&mut Settings)) {
        change(&mut self.settings);
        self.write(SETTINGS, &self.settings);
    }

    // ---- the last queue

    pub fn save_queue(&self, queue: Option<&SavedQueue>) {
        match queue {
            Some(queue) => self.write(QUEUE, queue),
            None => {
                self.remove(QUEUE);
                self.remove(QUEUE_POSITION);
            }
        }
    }

    pub fn save_position(&self, position: QueuePosition) {
        self.write(QUEUE_POSITION, &position);
    }

    pub fn load_queue(&self) -> Option<(SavedQueue, QueuePosition)> {
        let queue: SavedQueue = self.read(QUEUE)?;
        if queue.items.is_empty() {
            return None;
        }
        let mut position: QueuePosition = self.read(QUEUE_POSITION).unwrap_or_default();
        position.index = position.index.min(queue.items.len() - 1);
        position.position_secs = position.position_secs.max(0.0);
        Some((queue, position))
    }

    // ---- playback events

    /// Logs a playback event; returns it, to send to the app.
    pub fn record_event(&mut self, kind: &str, item_id: i64, position_ms: i64) -> Value {
        let event = json!({ "id": new_id(), "type": kind, "itemId": item_id, "positionMs": position_ms.max(0), "atMs": now_ms() });
        self.events.push(event.clone());
        if self.events.len() > MAX_PLAYBACK_EVENTS {
            let extra = self.events.len() - MAX_PLAYBACK_EVENTS;
            self.events.drain(..extra);
        }
        self.write(PLAYBACK_EVENTS, &self.events);
        event
    }

    pub fn pending_events(&self) -> Vec<Value> {
        self.events.clone()
    }

    pub fn acknowledge_events(&mut self, ids: &[String]) {
        let ids: HashSet<&str> = ids.iter().map(String::as_str).collect();
        self.events.retain(|e| !ids.contains(e["id"].as_str().unwrap_or_default()));
        self.write(PLAYBACK_EVENTS, &self.events);
    }

    // ---- tracked lists

    /// Sets the list definitions; lists no longer defined are dropped.
    pub fn set_list_configs(&mut self, lists: Vec<ListConfig>) -> Result<(), String> {
        let mut ids = HashSet::new();
        for list in &lists {
            if list.id.is_empty() {
                return Err("every list needs an id".into());
            }
            if !ids.insert(list.id.clone()) {
                return Err(format!("duplicate list id {}", list.id));
            }
            if list.track != "folder" && list.track != "item" {
                return Err(format!("list {}: track must be folder or item", list.id));
            }
            if list.limit == 0 {
                return Err(format!("list {}: limit must be > 0", list.id));
            }
            if !list.count_after_seconds.is_finite() || list.count_after_seconds < 0.0 {
                return Err(format!("list {}: countAfterSeconds must be >= 0", list.id));
            }
        }
        let tracked = &mut self.tracked;
        tracked.lists = lists
            .iter()
            .map(|l| (l.id.clone(), tracked.lists.remove(&l.id).unwrap_or_default().into_iter().take(l.limit).collect()))
            .collect();
        tracked.changes.retain(|id, _| ids.contains(id));
        tracked.configs = lists;
        self.write(TRACKED_LISTS, &self.tracked);
        Ok(())
    }

    pub fn list_configs(&self) -> &[ListConfig] {
        &self.tracked.configs
    }

    pub fn list_entries(&self, id: &str) -> Vec<ListEntry> {
        self.tracked.lists.get(id).cloned().unwrap_or_default()
    }

    /// Sets a list from the app: `merge` combines with what's there (newest time per id wins).
    /// Not reported back as a change.
    pub fn set_list(&mut self, id: &str, entries: Vec<ListEntry>, merge: bool) -> Result<Vec<ListEntry>, String> {
        let limit = self.tracked.configs.iter().find(|c| c.id == id).ok_or_else(|| format!("unknown list {id}"))?.limit;
        let entries: Vec<ListEntry> = entries.into_iter().map(normalize_entry).collect();
        let base = if merge { self.list_entries(id) } else { Vec::new() };
        let result = combine(base.into_iter().chain(entries).collect(), limit);
        self.tracked.lists.insert(id.to_string(), result.clone());
        self.write(TRACKED_LISTS, &self.tracked);
        Ok(result)
    }

    /// Records that `value` (a folder or item id) was played now. Returns the change to send to
    /// the app, or None when the list stayed the same.
    pub fn record_list(&mut self, list_id: &str, value: Value) -> Option<Value> {
        let limit = self.tracked.configs.iter().find(|c| c.id == list_id)?.limit;
        let now = now_ms();
        let current = self.list_entries(list_id);
        let updated = combine(current.iter().cloned().chain([ListEntry { id: value, played_at_ms: now }]).collect(), limit);
        if updated == current {
            return None;
        }
        self.tracked.lists.insert(list_id.to_string(), updated.clone());
        let change = json!({
            "id": new_id(),
            "listId": list_id,
            "entries": updated,
            "ids": updated.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
            "changedAtMs": now,
        });
        // Only the latest state of each list waits for the app.
        self.tracked.changes.insert(list_id.to_string(), change.clone());
        self.write(TRACKED_LISTS, &self.tracked);
        Some(change)
    }

    /// Unacknowledged changes, the latest per list, oldest first.
    pub fn pending_list_changes(&self) -> Vec<Value> {
        let mut changes: Vec<Value> = self.tracked.changes.values().cloned().collect();
        changes.sort_by_key(|c| c["changedAtMs"].as_i64().unwrap_or(0));
        changes
    }

    pub fn acknowledge_list_changes(&mut self, ids: &[String]) {
        let ids: HashSet<&str> = ids.iter().map(String::as_str).collect();
        self.tracked.changes.retain(|_, c| !ids.contains(c["id"].as_str().unwrap_or_default()));
        self.write(TRACKED_LISTS, &self.tracked);
    }

    // ---- item progress

    pub fn progress(&self, item_ids: Option<&[i64]>) -> Vec<Value> {
        let entry = |id: i64, p: &Progress| json!({ "itemId": id, "progress": p.progress, "positionMs": p.position_ms, "updatedAtMs": p.updated_at_ms });
        match item_ids {
            Some(ids) => ids.iter().filter_map(|id| self.progress.get(id).map(|p| entry(*id, p))).collect(),
            None => self.progress.iter().map(|(id, p)| entry(*id, p)).collect(),
        }
    }

    /// Playback of `item_id` reached `position_ms` of `duration_ms`; `completed` counts as played.
    pub fn record_progress(&mut self, item_id: i64, position_ms: i64, duration_ms: i64, completed: bool) {
        if !completed && duration_ms <= 0 {
            return;
        }
        let progress = if completed { 1.0 } else { (position_ms as f64 / duration_ms as f64).clamp(0.0, 1.0) };
        self.progress.insert(
            item_id,
            Progress {
                progress: if progress >= PLAYED_AT { 1.0 } else { progress },
                position_ms: if completed { duration_ms.max(position_ms) } else { position_ms.max(0) },
                updated_at_ms: now_ms(),
            },
        );
        self.save_progress();
    }

    /// Sets progress from the app: `merge` keeps whichever version of each item is newer.
    /// Returns the stored result for the given items.
    pub fn set_progress(&mut self, entries: Vec<(i64, Progress)>, merge: bool) -> Vec<Value> {
        let ids: Vec<i64> = entries.iter().map(|(id, _)| *id).collect();
        for (id, entry) in entries {
            let newer = self.progress.get(&id).is_none_or(|current| entry.updated_at_ms > current.updated_at_ms);
            if !merge || newer {
                self.progress.insert(id, entry);
            }
        }
        self.save_progress();
        self.progress(Some(&ids))
    }

    fn save_progress(&self) {
        let by_key: HashMap<String, &Progress> = self.progress.iter().map(|(k, v)| (k.to_string(), v)).collect();
        self.write(ITEM_PROGRESS, &by_key);
    }

    // ---- the progress checkpoint (the current item and time)

    pub fn save_checkpoint(&self, item_id: i64, current_time: f64, status: &str) {
        self.write(CHECKPOINT, &json!({ "id": item_id, "currentTime": current_time, "updatedAtMs": now_ms(), "status": status }));
    }

    pub fn checkpoint(&self) -> Option<Value> {
        let checkpoint: Value = self.read(CHECKPOINT)?;
        let valid = checkpoint["id"].as_i64().is_some_and(|id| id > 0) && checkpoint["currentTime"].as_f64().is_some_and(|t| t > 0.0);
        valid.then_some(checkpoint)
    }

    pub fn clear_checkpoint(&self) {
        self.remove(CHECKPOINT);
    }
}

/// Item ids as numbers (integers), folder ids as strings.
fn normalize_entry(entry: ListEntry) -> ListEntry {
    let id = match entry.id {
        Value::Number(n) => n.as_i64().map(Value::from).unwrap_or(Value::Number(n)),
        Value::String(s) => Value::String(s),
        other => Value::String(other.to_string()),
    };
    ListEntry { id, ..entry }
}

/// One entry per id with its newest time, newest first, at most `limit`.
pub fn combine(entries: Vec<ListEntry>, limit: usize) -> Vec<ListEntry> {
    let mut by_key: HashMap<String, ListEntry> = HashMap::new();
    for entry in entries {
        let key = entry.key();
        let keep = by_key.get(&key).is_none_or(|current| entry.played_at_ms > current.played_at_ms);
        if keep {
            by_key.insert(key, entry);
        }
    }
    let mut result: Vec<ListEntry> = by_key.into_values().collect();
    // Newest first; ties in a fixed order, so merging is the same whichever side comes first.
    result.sort_by(|a, b| b.played_at_ms.cmp(&a.played_at_ms).then_with(|| a.key().cmp(&b.key())));
    result.truncate(limit);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: impl Into<Value>, at: i64) -> ListEntry {
        ListEntry { id: id.into(), played_at_ms: at }
    }

    fn ids(entries: &[ListEntry]) -> Vec<Value> {
        entries.iter().map(|e| e.id.clone()).collect()
    }

    // ---- TrackedListsTest

    #[test]
    fn merge_keeps_the_newest_time_per_id_and_sorts_newest_first() {
        // This device played pl-1 at 100 and pl-2 at 300; another device played pl-1 at 400 and pl-3 at 200.
        let local = vec![entry("pl-2", 300), entry("pl-1", 100)];
        let remote = vec![entry("pl-1", 400), entry("pl-3", 200)];
        let merged = combine(local.into_iter().chain(remote).collect(), 10);
        assert_eq!(ids(&merged), vec![json!("pl-1"), json!("pl-2"), json!("pl-3")]);
        assert_eq!(merged[0].played_at_ms, 400);
    }

    #[test]
    fn merge_is_the_same_whichever_side_comes_first() {
        let a = vec![entry(1, 10), entry(2, 30)];
        let b = vec![entry(2, 20), entry(3, 40)];
        let ab = combine(a.iter().chain(&b).cloned().collect(), 10);
        let ba = combine(b.iter().chain(&a).cloned().collect(), 10);
        assert_eq!(ids(&ab), ids(&ba));
    }

    #[test]
    fn merge_caps_at_the_limit() {
        let entries: Vec<ListEntry> = (1..=8).map(|i| entry(i, i * 10)).collect();
        assert_eq!(ids(&combine(entries, 5)), vec![json!(8), json!(7), json!(6), json!(5), json!(4)]);
    }

    // ---- saving

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("native-audio-test-{name}-{}", new_id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn everything_is_there_again_after_reopening() {
        let dir = temp_dir("reopen");
        {
            let mut p = Persist::open(Some(dir.clone()));
            p.update_settings(|s| s.volume = Some(0.5));
            p.set_list_configs(vec![ListConfig { id: "recent".into(), track: "item".into(), limit: 3, count_after_seconds: 0.0 }]).unwrap();
            assert!(p.record_list("recent", json!(7)).is_some());
            assert!(p.record_list("recent", json!(7)).is_some()); // newer time: a change
            p.record_progress(7, 30_000, 60_000, false);
            p.record_event("start", 7, 0);
            p.save_queue(Some(&SavedQueue {
                items: vec![SavedItem { item: Item { src: "a.mp3".into(), id: Some(7), title: None, artist: None, artwork_url: None }, added: true }],
                order: vec![0],
                shuffle: false,
                repeat_mode: "all".into(),
                source_id: Some("pl".into()),
            }));
            p.save_position(QueuePosition { index: 5, position_secs: 12.5 });
        }
        let p = Persist::open(Some(dir.clone()));
        assert_eq!(p.settings().volume, Some(0.5));
        assert_eq!(ids(&p.list_entries("recent")), vec![json!(7)]);
        assert_eq!(p.pending_list_changes().len(), 1); // only the latest per list
        assert_eq!(p.progress(Some(&[7]))[0]["progress"], json!(0.5));
        assert_eq!(p.pending_events().len(), 1);
        let (queue, position) = p.load_queue().unwrap();
        assert!(queue.items[0].added);
        assert_eq!(queue.items[0].item.id, Some(7));
        assert_eq!(position.index, 0); // clamped to the queue
        assert_eq!(position.position_secs, 12.5);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn acknowledged_events_and_changes_are_gone() {
        let mut p = Persist::open(None);
        let event = p.record_event("complete", 1, 1000);
        p.set_list_configs(vec![ListConfig { id: "l".into(), track: "folder".into(), limit: 5, count_after_seconds: 0.0 }]).unwrap();
        let change = p.record_list("l", json!("pl-1")).unwrap();
        p.acknowledge_events(&[event["id"].as_str().unwrap().to_string()]);
        p.acknowledge_list_changes(&[change["id"].as_str().unwrap().to_string()]);
        assert!(p.pending_events().is_empty());
        assert!(p.pending_list_changes().is_empty());
    }

    #[test]
    fn progress_counts_as_played_near_the_end_and_merge_keeps_the_newer() {
        let mut p = Persist::open(None);
        p.record_progress(1, 99_000, 100_000, false);
        assert_eq!(p.progress(Some(&[1]))[0]["progress"], json!(1.0));
        let stored = p.progress.get(&1).copied().unwrap();
        let older = Progress { progress: 0.1, position_ms: 10, updated_at_ms: stored.updated_at_ms - 1 };
        p.set_progress(vec![(1, older)], true);
        assert_eq!(p.progress(Some(&[1]))[0]["progress"], json!(1.0));
        p.set_progress(vec![(1, older)], false);
        assert_eq!(p.progress(Some(&[1]))[0]["progress"], json!(0.1));
    }

    #[test]
    fn redefining_lists_drops_the_old_ones_and_applies_new_limits() {
        let mut p = Persist::open(None);
        let config = |id: &str, limit| ListConfig { id: id.into(), track: "item".into(), limit, count_after_seconds: 0.0 };
        p.set_list_configs(vec![config("a", 5), config("b", 5)]).unwrap();
        p.set_list("a", (1..=4).map(|i| entry(i, i)).collect(), false).unwrap();
        p.set_list("b", vec![entry(1, 1)], false).unwrap();
        p.set_list_configs(vec![config("a", 2)]).unwrap();
        assert_eq!(ids(&p.list_entries("a")), vec![json!(4), json!(3)]);
        assert!(p.list_entries("b").is_empty());
        assert!(p.set_list("b", vec![], false).is_err());
        assert!(p.set_list_configs(vec![config("x", 1), config("x", 1)]).is_err());
    }
}
