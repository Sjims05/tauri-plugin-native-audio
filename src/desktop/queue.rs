//! The play queue on desktop: the list, its play order (shuffled or not), repeat, and edits.
//! The same rules as the Android player (ShuffleOrderBuilder.kt, QueueOrder.kt, NativeAudioRuntime).
//!
//! Every entry has a key that never changes, so the engine and the queue keep agreeing on what
//! plays while entries are inserted, moved and removed around it.

use std::collections::{HashMap, HashSet};

use super::persist::{SavedItem, SavedQueue};
use super::player::Item;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    Off,
    All,
    One,
}

impl Repeat {
    pub fn parse(mode: &str) -> Option<Self> {
        match mode {
            "off" => Some(Self::Off),
            "all" => Some(Self::All),
            "one" => Some(Self::One),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::All => "all",
            Self::One => "one",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub key: u64,
    pub item: Item,
    /// Added with addToQueue: dropped when the queue repeats, unless repeatAddedTracks.
    pub added: bool,
}

/// Where updateQueue puts new songs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NewItems {
    /// At their place in the playlist (with shuffle: somewhere in the rest of this pass).
    #[default]
    InPlace,
    /// At the end (with shuffle: the end of the play order, shuffled among themselves).
    End,
    /// Right after the playing song, in playlist order.
    Next,
}

/// How updateQueue treats what changed (the defaults: remove what left, new items in place, the
/// playlist's order).
#[derive(Debug, Clone, Copy, Default)]
pub struct UpdateOptions {
    /// Songs no longer in the playlist stay in the queue where they are.
    pub keep_removed: bool,
    pub new_items: NewItems,
    /// Keep the queue's own list order instead of taking the playlist's new order (new songs are
    /// placed by `new_items`; in place: after the song before them in the playlist).
    pub keep_queue_order: bool,
}

/// What an update changed in the playlist.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UpdateSummary {
    /// Songs new to the playlist.
    pub added: usize,
    /// Songs no longer in the playlist (also when kept, or playing to the end).
    pub removed: usize,
}

pub struct Queue {
    entries: Vec<Entry>,
    /// List indices in play order.
    order: Vec<usize>,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub repeat_added: bool,
    next_key: u64,
    rng: Rng,
    /// The playable folder this queue was started from (setQueue's sourceId), for tracked lists.
    pub source_id: Option<String>,
    /// Goes up with every change, so the queue is saved only when it changed.
    revision: u64,
    /// Entries that left the playlist (update) while playing: dropped once playback moves on.
    pending_removal: HashSet<u64>,
    /// What the engine was told follows each entry (`next_for_engine`), newest last.
    told_engine: Vec<(u64, Option<u64>)>,
}

impl Default for Queue {
    fn default() -> Self {
        Self::new(Rng::from_time())
    }
}

impl Queue {
    pub fn new(rng: Rng) -> Self {
        Self {
            entries: Vec::new(),
            order: Vec::new(),
            shuffle: false,
            repeat: Repeat::Off,
            repeat_added: false,
            next_key: 1,
            rng,
            source_id: None,
            revision: 0,
            pending_removal: HashSet::new(),
            told_engine: Vec::new(),
        }
    }

    fn set_order(&mut self, order: Vec<usize>) {
        self.order = order;
        self.revision += 1;
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn set_repeat(&mut self, repeat: Repeat) {
        self.repeat = repeat;
        self.revision += 1;
    }

    /// The queue to save, or None when it's empty.
    pub fn snapshot(&self) -> Option<SavedQueue> {
        (!self.entries.is_empty()).then(|| SavedQueue {
            items: self.entries.iter().map(|e| SavedItem { item: e.item.clone(), added: e.added }).collect(),
            order: self.order.clone(),
            shuffle: self.shuffle,
            repeat_mode: self.repeat.name().to_string(),
            source_id: self.source_id.clone(),
        })
    }

    /// Loads a saved queue; returns the key of the entry at `index`.
    pub fn restore(&mut self, saved: SavedQueue, index: usize) -> Option<u64> {
        let added: Vec<bool> = saved.items.iter().map(|i| i.added).collect();
        let shuffle = saved.shuffle;
        self.shuffle = false;
        let key = self.set(saved.items.into_iter().map(|i| i.item).collect(), index);
        for (entry, added) in self.entries.iter_mut().zip(added) {
            entry.added = added;
        }
        self.shuffle = shuffle;
        self.repeat = Repeat::parse(&saved.repeat_mode).unwrap_or(Repeat::Off);
        self.source_id = saved.source_id;
        // The saved order if it still fits; otherwise a new one (shuffled from the current entry).
        let mut sorted = saved.order.clone();
        sorted.sort_unstable();
        if sorted == (0..self.entries.len()).collect::<Vec<_>>() {
            self.set_order(saved.order);
        } else {
            self.set_shuffle(shuffle, key);
        }
        key
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn order(&self) -> &[usize] {
        &self.order
    }

    pub fn index_of(&self, key: u64) -> Option<usize> {
        self.entries.iter().position(|e| e.key == key)
    }

    pub fn entry(&self, key: u64) -> Option<&Entry> {
        self.entries.iter().find(|e| e.key == key)
    }

    pub fn key_at(&self, index: usize) -> Option<u64> {
        self.entries.get(index).map(|e| e.key)
    }

    fn position_in_order(&self, key: u64) -> Option<usize> {
        let index = self.index_of(key)?;
        self.order.iter().position(|&i| i == index)
    }

    fn new_entries(&mut self, items: Vec<Item>, added: bool) -> Vec<Entry> {
        items
            .into_iter()
            .map(|item| {
                let key = self.next_key;
                self.next_key += 1;
                Entry { key, item, added }
            })
            .collect()
    }

    /// Replaces the queue; returns the key of the entry at `start` (clamped).
    pub fn set(&mut self, items: Vec<Item>, start: usize) -> Option<u64> {
        self.entries = self.new_entries(items, false);
        self.source_id = None;
        self.pending_removal.clear();
        self.told_engine.clear();
        if self.entries.is_empty() {
            self.set_order(Vec::new());
            return None;
        }
        let start = start.min(self.entries.len() - 1);
        let order = if self.shuffle { self.shuffled(Some(start), &HashSet::new()) } else { (0..self.entries.len()).collect() };
        self.set_order(order);
        self.key_at(start)
    }

    /// Shuffle on: a fresh order that starts with the current entry. Off: the list order.
    pub fn set_shuffle(&mut self, on: bool, current: Option<u64>) {
        self.shuffle = on;
        let first = current.and_then(|k| self.index_of(k));
        let order = if on { self.shuffled(first, &HashSet::new()) } else { (0..self.entries.len()).collect() };
        self.set_order(order);
    }

    fn shuffled(&mut self, first: Option<usize>, recent: &HashSet<usize>) -> Vec<usize> {
        let artists: Vec<Option<String>> = self.entries.iter().map(|e| e.item.artist.clone()).collect();
        build_shuffle(self.entries.len(), first, recent, |i| artists[i].clone(), &mut self.rng)
    }

    /// What plays after `key`, without changing anything. `auto`: the track ended on its own (repeat
    /// one plays it again); otherwise it's the next button (repeat one acts like off, as on Android).
    pub fn peek_next(&self, key: u64, auto: bool) -> Option<u64> {
        let pos = self.position_in_order(key)?;
        if auto && self.repeat == Repeat::One {
            return Some(key);
        }
        if pos + 1 < self.order.len() {
            return self.key_at(self.order[pos + 1]);
        }
        (self.repeat == Repeat::All).then(|| self.key_at(self.order[0])).flatten()
    }

    /// What follows `key` when it ends, for the engine (it asks a couple of seconds before the end).
    /// The answer is remembered, so a later edit knows what the engine prepared (`told_engine_after`).
    pub fn next_for_engine(&mut self, key: u64) -> Option<u64> {
        let next = self.peek_next(key, true);
        self.told_engine.push((key, next));
        if self.told_engine.len() > 16 {
            self.told_engine.remove(0);
        }
        next
    }

    /// What the engine was last told follows `key`, or None when it hasn't asked yet.
    pub fn told_engine_after(&self, key: u64) -> Option<Option<u64>> {
        self.told_engine.iter().rev().find(|(k, _)| *k == key).map(|(_, next)| *next)
    }

    /// The engine restarts `key` (it will ask what follows again).
    pub fn forget_told_engine(&mut self, key: u64) {
        self.told_engine.retain(|(k, _)| *k != key);
    }

    /// Playback moved from `from` to `to`. When that started the queue over (repeat all, from the last
    /// entry of the play order to the first): drops the added entries (unless repeat_added) and, with
    /// shuffle, builds a new order for the next pass that starts with `to` (Android's handleWrapLocked).
    pub fn on_transition(&mut self, from: u64, to: u64) {
        self.wrap(from, to);
        // Left the playlist (update) while it played: gone now that it's over.
        if from != to && self.pending_removal.remove(&from) {
            if let Some(index) = self.index_of(from) {
                self.remove_index(index);
            }
        }
    }

    fn wrap(&mut self, from: u64, to: u64) {
        if self.repeat != Repeat::All || self.order.len() < 2 {
            return;
        }
        let (Some(from_index), Some(to_index)) = (self.index_of(from), self.index_of(to)) else { return };
        if self.order.last() != Some(&from_index) || self.order.first() != Some(&to_index) {
            return;
        }
        if !self.repeat_added {
            let added: Vec<u64> = self.entries.iter().filter(|e| e.added && e.key != to).map(|e| e.key).collect();
            for key in added {
                if let Some(index) = self.index_of(key) {
                    self.remove_index(index);
                }
            }
        }
        if self.shuffle && self.order.len() >= 2 {
            // `to` opened the old pass, so it's the least recently played one.
            let recent: HashSet<usize> = recent_tail(&self.order).into_iter().collect();
            let first = self.index_of(to);
            let order = self.shuffled(first, &recent);
            self.set_order(order);
        }
    }

    /// The playlist the queue plays changed: `items` is its new content. Entries are matched by item
    /// id (the same id twice: in order), so what stays keeps its place in the play order, new items
    /// are added and missing ones removed. Entries added with addToQueue stay, after the entry they
    /// followed. The `current` entry keeps playing if it's no longer in the playlist, and leaves the
    /// queue when playback moves on (`is_leaving`). See UpdateOptions for the alternatives.
    pub fn update(&mut self, items: Vec<Item>, current: Option<u64>, options: UpdateOptions) -> UpdateSummary {
        let old = std::mem::take(&mut self.entries);
        let old_order: Vec<u64> = self.order.iter().map(|&i| old[i].key).collect();
        let mut used = vec![false; old.len()];
        // The new playlist: old entries (with the new details) where matched, new ones otherwise.
        let mut playlist: Vec<Entry> = Vec::with_capacity(items.len());
        let mut fresh: HashSet<u64> = HashSet::new();
        for item in items {
            let matched = item
                .id
                .and_then(|id| (0..old.len()).find(|&i| !used[i] && !old[i].added && old[i].item.id == Some(id)));
            let key = match matched {
                Some(i) => {
                    used[i] = true;
                    old[i].key
                }
                None => {
                    let key = self.next_key;
                    self.next_key += 1;
                    fresh.insert(key);
                    key
                }
            };
            playlist.push(Entry { key, item, added: false });
        }
        let summary = UpdateSummary {
            added: fresh.len(),
            removed: (0..old.len()).filter(|&i| !used[i] && !old[i].added).count(),
        };
        // Old entries that stay although they're not in the playlist: added ones, the playing one
        // (until it's over), and with keep_removed all of them.
        let kept = |i: usize| !used[i] && (old[i].added || options.keep_removed || Some(old[i].key) == current);
        let keep_entry = |entry: &Entry, pending: &mut HashSet<u64>| {
            let mut entry = entry.clone();
            if !entry.added && !options.keep_removed {
                entry.added = true;
                pending.insert(entry.key);
            }
            entry
        };
        let mut pending = std::mem::take(&mut self.pending_removal);
        let (mut list, new): (Vec<Entry>, Vec<Entry>) = if options.keep_queue_order {
            // The queue's order: old entries that stay, where they were.
            let mut details: HashMap<u64, Entry> = playlist.iter().filter(|e| !fresh.contains(&e.key)).map(|e| (e.key, e.clone())).collect();
            let mut list: Vec<Entry> = (0..old.len())
                .filter_map(|i| if used[i] { details.remove(&old[i].key) } else if kept(i) { Some(keep_entry(&old[i], &mut pending)) } else { None })
                .collect();
            let mut new = Vec::new();
            for (p, entry) in playlist.iter().enumerate() {
                if !fresh.contains(&entry.key) {
                    continue;
                }
                if options.new_items == NewItems::InPlace {
                    // After the song before it in the playlist (that's in the queue).
                    let at = (0..p).rev().find_map(|q| list.iter().position(|e| e.key == playlist[q].key)).map_or(0, |i| i + 1);
                    list.insert(at, entry.clone());
                } else {
                    new.push(entry.clone());
                }
            }
            (list, new)
        } else {
            // The playlist's order; kept entries after the nearest entry before them (in the old list).
            let (mut list, new): (Vec<Entry>, Vec<Entry>) =
                playlist.into_iter().partition(|e| options.new_items == NewItems::InPlace || !fresh.contains(&e.key));
            for i in 0..old.len() {
                if !kept(i) {
                    continue;
                }
                let at = (0..i).rev().find_map(|j| list.iter().position(|e| e.key == old[j].key)).map_or(0, |p| p + 1);
                let entry = keep_entry(&old[i], &mut pending);
                list.insert(at, entry);
            }
            (list, new)
        };
        match options.new_items {
            NewItems::InPlace => {}
            NewItems::End => list.extend(new),
            NewItems::Next => {
                let at = current.and_then(|c| list.iter().position(|e| e.key == c)).map_or(list.len(), |p| p + 1);
                list.splice(at..at, new);
            }
        }
        pending.retain(|key| list.iter().any(|e| e.key == *key));
        self.pending_removal = pending;
        self.entries = list;

        let order = if self.shuffle {
            let index: HashMap<u64, usize> = self.entries.iter().enumerate().map(|(i, e)| (e.key, i)).collect();
            let mut order: Vec<usize> = old_order.iter().filter_map(|k| index.get(k).copied()).collect();
            let start = current.and_then(|c| order.iter().position(|&i| self.entries[i].key == c)).map_or(0, |p| p + 1);
            // New entries in list order (so "next" keeps the playlist's order).
            let mut new: Vec<usize> = (0..self.entries.len()).filter(|&i| fresh.contains(&self.entries[i].key)).collect();
            match options.new_items {
                NewItems::Next => {
                    order.splice(start..start, new);
                }
                NewItems::End => {
                    self.rng.shuffle(&mut new);
                    order.extend(new);
                }
                NewItems::InPlace => {
                    self.rng.shuffle(&mut new);
                    for i in new {
                        let at = start + self.rng.below(order.len() - start + 1);
                        order.insert(at, i);
                    }
                }
            }
            order
        } else {
            (0..self.entries.len()).collect()
        };
        self.set_order(order);
        summary
    }

    /// The entry left the playlist (update) and leaves the queue once playback moves on.
    pub fn is_leaving(&self, key: u64) -> bool {
        self.pending_removal.contains(&key)
    }

    /// The previous button's target: the entry before `key` in play order (repeat all wraps around).
    pub fn previous(&self, key: u64) -> Option<u64> {
        let pos = self.position_in_order(key)?;
        if pos > 0 {
            return self.key_at(self.order[pos - 1]);
        }
        (self.repeat == Repeat::All && self.order.len() > 1).then(|| self.key_at(*self.order.last()?)).flatten()
    }

    /// Adds after the current entry (`play_next`, in the list and in play order) or at the end.
    /// Adding to an empty queue makes it the queue (not marked as added). Returns the first new key.
    pub fn add(&mut self, items: Vec<Item>, play_next: bool, current: Option<u64>) -> Option<u64> {
        if items.is_empty() {
            return None;
        }
        if self.entries.is_empty() {
            return self.set(items, 0);
        }
        let count = items.len();
        let new = self.new_entries(items, true);
        let first_key = new[0].key;
        let current_index = current.and_then(|k| self.index_of(k));
        let at = match (play_next, current_index) {
            (true, Some(index)) => index + 1,
            _ => self.entries.len(),
        };
        let play_position = match (play_next, current.and_then(|k| self.position_in_order(k))) {
            (true, Some(pos)) => pos + 1,
            _ => self.order.len(),
        };
        self.entries.splice(at..at, new);
        let order = order_insert(&self.order, at, count, play_position);
        self.set_order(order);
        Some(first_key)
    }

    /// Removes the entry at list `index`; returns its key.
    pub fn remove(&mut self, index: usize) -> Option<u64> {
        let key = self.key_at(index)?;
        self.remove_index(index);
        Some(key)
    }

    fn remove_index(&mut self, index: usize) {
        self.entries.remove(index);
        let order = order_remove(&self.order, index);
        self.set_order(order);
    }

    /// Moves the list entry at `from` to `to`. Without shuffle the play order is the list, so it
    /// changes too; with shuffle on it stays the same (only the list position changes).
    pub fn move_entry(&mut self, from: usize, to: usize) -> bool {
        if from >= self.entries.len() || to >= self.entries.len() {
            return false;
        }
        let entry = self.entries.remove(from);
        self.entries.insert(to, entry);
        let order = if self.shuffle { order_move(&self.order, from, to) } else { (0..self.entries.len()).collect() };
        self.set_order(order);
        true
    }
}

// ---- QueueOrder.kt: keeping a play order in step with edits to the list.

/// `count` entries were inserted into the list at `at`; they go to `play_position` in the order.
pub fn order_insert(order: &[usize], at: usize, count: usize, play_position: usize) -> Vec<usize> {
    let mut shifted: Vec<usize> = order.iter().map(|&i| if i >= at { i + count } else { i }).collect();
    let position = play_position.min(shifted.len());
    shifted.splice(position..position, at..at + count);
    shifted
}

/// The list entry at `at` was removed.
pub fn order_remove(order: &[usize], at: usize) -> Vec<usize> {
    order.iter().filter(|&&i| i != at).map(|&i| if i > at { i - 1 } else { i }).collect()
}

/// The list entry at `from` moved to `to`; its place in the play order doesn't change.
pub fn order_move(order: &[usize], from: usize, to: usize) -> Vec<usize> {
    order.iter().map(|&i| moved_index(i, from, to)).collect()
}

/// Where list index `index` ends up after moving `from` to `to`.
pub fn moved_index(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        to
    } else if from < to && index > from && index <= to {
        index - 1
    } else if from > to && index >= to && index < from {
        index + 1
    } else {
        index
    }
}

// ---- ShuffleOrderBuilder.kt: shuffled orders that feel random to a listener.

/// A shuffled play order of `count` indices: `first` first; the `recent` ones (the end of the
/// previous pass) kept out of the start; where possible, no artist twice in a row.
pub fn build_shuffle(
    count: usize,
    first: Option<usize>,
    recent: &HashSet<usize>,
    artist_of: impl Fn(usize) -> Option<String>,
    rng: &mut Rng,
) -> Vec<usize> {
    if count == 0 {
        return Vec::new();
    }
    let pinned = first.filter(|&f| f < count);
    let rest: Vec<usize> = (0..count).filter(|&i| Some(i) != pinned).collect();
    let mut fresh: Vec<usize> = rest.iter().copied().filter(|i| !recent.contains(i)).collect();
    let stale: Vec<usize> = rest.iter().copied().filter(|i| recent.contains(i)).collect();
    rng.shuffle(&mut fresh);

    let head_len = fresh.len().min(stale.len());
    let mut head: Vec<usize> = fresh[..head_len].to_vec();
    let mut tail: Vec<usize> = fresh[head_len..].iter().copied().chain(stale).collect();
    rng.shuffle(&mut tail);

    let mut artists: HashMap<usize, Option<String>> = HashMap::new();
    let mut normalized = |i: usize| -> Option<String> {
        artists
            .entry(i)
            .or_insert_with(|| artist_of(i).map(|a| a.trim().to_lowercase()).filter(|a| !a.is_empty()))
            .clone()
    };
    let pinned_artist = pinned.and_then(&mut normalized);
    spread_artists(&mut head, pinned_artist.clone(), &mut normalized);
    let before_tail = head.last().map_or(pinned_artist, |&i| normalized(i));
    spread_artists(&mut tail, before_tail, &mut normalized);

    pinned.into_iter().chain(head).chain(tail).collect()
}

/// The last ~20% of `order`: what the next pass shouldn't start with.
pub fn recent_tail(order: &[usize]) -> Vec<usize> {
    if order.len() <= 2 {
        return Vec::new();
    }
    let n = (order.len() / 5).max(1);
    order[order.len() - n..].to_vec()
}

/// Reorders the shuffled `items` so no two neighbours share an artist, when possible: takes the
/// first one whose artist differs from the previous, except when one artist fills more than half
/// of what's left, which then has to go now or its tracks end up back to back at the end.
fn spread_artists(items: &mut Vec<usize>, previous_artist: Option<String>, artist_of: &mut impl FnMut(usize) -> Option<String>) {
    let mut remaining = std::mem::take(items);
    let mut counts: HashMap<String, usize> = HashMap::new();
    for &i in &remaining {
        if let Some(a) = artist_of(i) {
            *counts.entry(a).or_default() += 1;
        }
    }
    let mut previous = previous_artist;
    while !remaining.is_empty() {
        let left = remaining.len();
        let forced = counts
            .iter()
            .filter(|(artist, &n)| Some(*artist) != previous.as_ref() && n * 2 > left)
            .map(|(artist, _)| artist.clone())
            .min(); // deterministic choice if several (only possible with exact ties)
        let pick = match &forced {
            Some(f) => remaining.iter().position(|&i| artist_of(i).as_ref() == Some(f)).unwrap_or(0),
            None => remaining
                .iter()
                .position(|&i| previous.is_none() || artist_of(i) != previous)
                .unwrap_or(0),
        };
        let index = remaining.remove(pick);
        let artist = artist_of(index);
        if let Some(a) = &artist {
            if let Some(n) = counts.get_mut(a) {
                *n -= 1;
            }
        }
        items.push(index);
        previous = artist;
    }
}

/// A small random generator (xorshift64*), so shuffling needs no extra dependency.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn from_time() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64);
        Self::new(nanos ^ 0x9E37_79B9_7F4A_7C15)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i + 1);
            items.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, artist: &str) -> Item {
        Item { src: format!("{name}.mp3"), id: None, title: Some(name.into()), artist: Some(artist.into()), artwork_url: None }
    }

    fn with_id(name: &str, id: i64) -> Item {
        Item { id: Some(id), ..item(name, name) }
    }

    fn queue(names: &[&str]) -> Queue {
        let mut q = Queue::new(Rng::new(42));
        q.set(names.iter().map(|n| item(n, n)).collect(), 0);
        q
    }

    fn titles(q: &Queue) -> Vec<String> {
        q.order().iter().map(|&i| q.entries()[i].item.title.clone().unwrap()).collect()
    }

    fn sorted(mut v: Vec<usize>) -> Vec<usize> {
        v.sort();
        v
    }

    // ---- ShuffleOrderBuilderTest

    #[test]
    fn orders_are_permutations_starting_with_first() {
        for seed in 1..200 {
            let order = build_shuffle(25, Some(7), &HashSet::new(), |_| None, &mut Rng::new(seed));
            assert_eq!(sorted(order.clone()), (0..25).collect::<Vec<_>>());
            assert_eq!(order[0], 7);
        }
    }

    #[test]
    fn recent_tracks_stay_out_of_the_start_of_the_next_pass() {
        for seed in 1..200 {
            let mut rng = Rng::new(seed);
            let previous = build_shuffle(50, None, &HashSet::new(), |_| None, &mut rng);
            let recent: HashSet<usize> = recent_tail(&previous).into_iter().collect();
            assert_eq!(recent.len(), 10);
            let next = build_shuffle(50, Some(previous[0]), &recent, |_| None, &mut rng);
            assert_eq!(sorted(next.clone()), (0..50).collect::<Vec<_>>());
            // Slot 0 is the pinned track; the next recent.len() slots must not be recent.
            assert!(next[1..=recent.len()].iter().all(|i| !recent.contains(i)), "seed {seed}");
        }
    }

    #[test]
    fn same_artist_is_not_played_back_to_back_when_avoidable() {
        // 20 tracks, 4 artists with 5 tracks each.
        let artist_of = |i: usize| Some(format!("Artist {}", i % 4));
        for seed in 1..200 {
            let order = build_shuffle(20, Some(0), &HashSet::new(), artist_of, &mut Rng::new(seed));
            for i in 1..order.len() {
                assert_ne!(artist_of(order[i]), artist_of(order[i - 1]), "seed {seed}: {order:?}");
            }
        }
    }

    #[test]
    fn handles_tiny_queues() {
        assert!(build_shuffle(0, None, &HashSet::new(), |_| None, &mut Rng::new(1)).is_empty());
        assert_eq!(build_shuffle(1, Some(0), &HashSet::new(), |_| None, &mut Rng::new(1)), vec![0]);
        assert!(recent_tail(&[1, 0]).is_empty());
    }

    // ---- QueueOrderTest. List: [A, B, C, D] = indices 0..3, shuffled play order C, A, D, B.
    const ORDER: [usize; 4] = [2, 0, 3, 1];

    #[test]
    fn play_next_goes_right_after_the_current_track_in_play_order() {
        // Current is A (index 0, play position 1); X goes to list index 1 and play position 2.
        assert_eq!(order_insert(&ORDER, 1, 1, 2), vec![3, 0, 1, 4, 2]);
    }

    #[test]
    fn add_to_end_goes_to_the_end_of_the_play_order() {
        assert_eq!(order_insert(&ORDER, 4, 2, ORDER.len()), vec![2, 0, 3, 1, 4, 5]);
    }

    #[test]
    fn remove_keeps_the_rest_of_the_order() {
        assert_eq!(order_remove(&ORDER, 1), vec![1, 0, 2]);
    }

    #[test]
    fn move_changes_list_positions_but_not_play_order() {
        assert_eq!(order_move(&ORDER, 0, 3), vec![1, 3, 2, 0]);
        assert_eq!(order_move(&[1, 3, 2, 0], 3, 0), ORDER.to_vec());
    }

    #[test]
    fn moved_index_matches_a_list_move() {
        let list = ["A", "B", "C", "D", "E"];
        for from in 0..list.len() {
            for to in 0..list.len() {
                let mut moved = list.to_vec();
                let x = moved.remove(from);
                moved.insert(to, x);
                for i in 0..list.len() {
                    assert_eq!(list[i], moved[moved_index(i, from, to)]);
                }
            }
        }
    }

    // ---- The queue

    #[test]
    fn next_and_previous_follow_the_play_order_and_repeat() {
        let mut q = queue(&["A", "B", "C"]);
        let [a, b, c] = [q.key_at(0).unwrap(), q.key_at(1).unwrap(), q.key_at(2).unwrap()];
        assert_eq!(q.peek_next(a, true), Some(b));
        assert_eq!(q.peek_next(c, true), None); // repeat off: the end
        assert_eq!(q.previous(a), None);
        q.repeat = Repeat::All;
        assert_eq!(q.peek_next(c, true), Some(a));
        assert_eq!(q.previous(a), Some(c));
        q.repeat = Repeat::One;
        assert_eq!(q.peek_next(b, true), Some(b)); // ended on its own: again
        assert_eq!(q.peek_next(b, false), Some(c)); // the next button still moves on
        assert_eq!(q.peek_next(c, false), None); // and acts like repeat off at the end
    }

    #[test]
    fn shuffle_starts_with_the_current_entry_and_off_restores_the_list() {
        let mut q = queue(&["A", "B", "C", "D", "E", "F"]);
        let c = q.key_at(2).unwrap();
        q.set_shuffle(true, Some(c));
        assert_eq!(q.order()[0], 2);
        assert_eq!(sorted(q.order().to_vec()), vec![0, 1, 2, 3, 4, 5]);
        q.set_shuffle(false, Some(c));
        assert_eq!(q.order(), &[0, 1, 2, 3, 4, 5]);
        assert_eq!(q.peek_next(c, false), q.key_at(3)); // continues in list order from C
    }

    #[test]
    fn play_next_and_add_to_end() {
        let mut q = queue(&["A", "B", "C"]);
        let a = q.key_at(0);
        q.add(vec![item("X", "x")], true, a);
        q.add(vec![item("Y", "y")], false, a);
        assert_eq!(titles(&q), ["A", "X", "B", "C", "Y"]);
        assert!(q.entries()[1].added && q.entries()[4].added && !q.entries()[0].added);
    }

    #[test]
    fn wrapping_drops_added_entries_unless_repeat_added() {
        let mut q = queue(&["A", "B"]);
        q.repeat = Repeat::All;
        let a = q.key_at(0);
        q.add(vec![item("X", "x")], true, a); // A X B
        let b = q.key_at(2).unwrap();
        assert_eq!(q.peek_next(b, true), a); // repeat all: back to A
        q.on_transition(b, a.unwrap());
        assert_eq!(titles(&q), ["A", "B"]);

        let mut q = queue(&["A", "B"]);
        q.repeat = Repeat::All;
        q.repeat_added = true;
        let a = q.key_at(0);
        q.add(vec![item("X", "x")], true, a);
        let b = q.key_at(2).unwrap();
        q.on_transition(b, a.unwrap());
        assert_eq!(titles(&q), ["A", "X", "B"]);
    }

    #[test]
    fn an_added_entry_that_ends_the_pass_is_dropped_too() {
        let mut q = queue(&["A", "B"]);
        q.repeat = Repeat::All;
        q.add(vec![item("X", "x")], false, q.key_at(0)); // A B X, X last
        let x = q.key_at(2).unwrap();
        let a = q.peek_next(x, true).unwrap();
        assert_eq!(Some(a), q.key_at(0));
        q.on_transition(x, a);
        assert_eq!(titles(&q), ["A", "B"]);
    }

    #[test]
    fn only_a_wrap_changes_the_queue() {
        let mut q = queue(&["A", "B", "C"]);
        q.repeat = Repeat::All;
        q.add(vec![item("X", "x")], false, q.key_at(0));
        let [a, c] = [q.key_at(0).unwrap(), q.key_at(2).unwrap()];
        q.on_transition(a, c); // a skip, not a wrap
        q.on_transition(c, a); // C isn't last (X is)
        assert_eq!(q.len(), 4);
        q.repeat = Repeat::Off;
        let x = q.key_at(3).unwrap();
        q.on_transition(x, a); // repeat off: picking A again isn't a wrap
        assert_eq!(q.len(), 4);
    }

    #[test]
    fn shuffle_reshuffles_on_each_pass_and_keeps_the_end_away_from_the_start() {
        let names: Vec<String> = (0..30).map(|i| format!("T{i}")).collect();
        let mut q = Queue::new(Rng::new(7));
        q.set(names.iter().map(|n| item(n, n)).collect(), 0);
        q.repeat = Repeat::All;
        q.set_shuffle(true, q.key_at(0));
        let first_pass = q.order().to_vec();
        let last = q.key_at(*first_pass.last().unwrap()).unwrap();
        let recent: HashSet<usize> = recent_tail(&first_pass).into_iter().collect();
        let next = q.peek_next(last, true).unwrap();
        assert_eq!(q.index_of(next), Some(first_pass[0]));
        q.on_transition(last, next);
        assert_eq!(q.order()[0], first_pass[0]); // the new pass starts with what's playing
        assert_ne!(q.order(), first_pass.as_slice());
        assert!(q.order()[1..=recent.len()].iter().all(|i| !recent.contains(i)));
    }

    #[test]
    fn a_saved_queue_comes_back_the_same() {
        let mut q = queue(&["A", "B", "C", "D"]);
        q.set_shuffle(true, q.key_at(1));
        q.set_repeat(Repeat::All);
        q.add(vec![item("X", "x")], true, q.key_at(1));
        q.source_id = Some("pl".into());
        let saved = q.snapshot().unwrap();
        let mut restored = Queue::new(Rng::new(1));
        let key = restored.restore(saved, 2);
        assert_eq!(restored.index_of(key.unwrap()), Some(2));
        assert_eq!(titles(&restored), titles(&q));
        assert_eq!(restored.repeat, Repeat::All);
        assert!(restored.shuffle && restored.entries()[2].added);
        assert_eq!(restored.source_id.as_deref(), Some("pl"));
        assert!(Queue::new(Rng::new(1)).snapshot().is_none());
    }

    #[test]
    fn update_keeps_what_stays_adds_new_and_removes_missing() {
        let mut q = Queue::new(Rng::new(3));
        q.set(["A", "B", "C", "D"].iter().enumerate().map(|(i, n)| with_id(n, i as i64)).collect(), 0);
        let [a, b, c] = [q.key_at(0).unwrap(), q.key_at(1).unwrap(), q.key_at(2).unwrap()];
        // B plays; the playlist is now A, C, E, B (D removed, E new, order changed).
        q.update(vec![with_id("A", 0), with_id("C", 2), with_id("E", 4), with_id("B", 1)], Some(b), UpdateOptions::default());
        assert_eq!(titles(&q), ["A", "C", "E", "B"]);
        assert_eq!([q.key_at(0), q.key_at(1), q.key_at(3)], [Some(a), Some(c), Some(b)]);
        assert_eq!(q.peek_next(b, true), None);
    }

    #[test]
    fn update_lets_the_playing_entry_finish_when_it_left_the_playlist() {
        let mut q = Queue::new(Rng::new(3));
        q.set(["A", "B", "C"].iter().enumerate().map(|(i, n)| with_id(n, i as i64)).collect(), 0);
        let b = q.key_at(1).unwrap();
        q.update(vec![with_id("A", 0), with_id("C", 2)], Some(b), UpdateOptions::default());
        assert_eq!(titles(&q), ["A", "B", "C"]); // B stays where it was while it plays
        assert!(q.is_leaving(b));
        let c = q.peek_next(b, true).unwrap();
        q.on_transition(b, c);
        assert_eq!(titles(&q), ["A", "C"]);
    }

    #[test]
    fn update_keeps_added_entries_after_the_entry_they_followed() {
        let mut q = Queue::new(Rng::new(3));
        q.set(["A", "B", "C"].iter().enumerate().map(|(i, n)| with_id(n, i as i64)).collect(), 0);
        q.add(vec![with_id("X", 9)], true, q.key_at(0)); // A X B C
        q.update(vec![with_id("C", 2), with_id("A", 0), with_id("B", 1)], q.key_at(0), UpdateOptions::default());
        assert_eq!(titles(&q), ["C", "A", "X", "B"]);
        assert!(q.entries()[2].added);
    }

    #[test]
    fn update_can_keep_removed_songs_and_put_new_ones_at_the_end() {
        let mut q = Queue::new(Rng::new(3));
        q.set(["A", "B", "C"].iter().enumerate().map(|(i, n)| with_id(n, i as i64)).collect(), 0);
        let options = UpdateOptions { keep_removed: true, new_items: NewItems::End, ..Default::default() };
        q.update(vec![with_id("N", 9), with_id("C", 2), with_id("A", 0)], q.key_at(0), options);
        assert_eq!(titles(&q), ["C", "A", "B", "N"]); // B kept (after A), N at the end
        assert!(!q.entries()[2].added && !q.is_leaving(q.key_at(2).unwrap()));

        // With shuffle: new ones at the end of the play order.
        let mut q = Queue::new(Rng::new(4));
        q.set((0..6).map(|i| with_id(&format!("T{i}"), i)).collect(), 0);
        q.set_shuffle(true, q.key_at(0));
        let before: Vec<u64> = q.order().iter().map(|&i| q.entries()[i].key).collect();
        let items = (0..6).map(|i| with_id(&format!("T{i}"), i)).chain([with_id("N1", 10), with_id("N2", 11)]).collect();
        q.update(items, q.key_at(0), UpdateOptions { new_items: NewItems::End, ..Default::default() });
        let after: Vec<u64> = q.order().iter().map(|&i| q.entries()[i].key).collect();
        assert_eq!(after[..6], before[..]);
    }

    #[test]
    fn update_can_play_new_songs_next_keep_the_queue_order_and_reports_changes() {
        let mut q = Queue::new(Rng::new(3));
        q.set(["A", "B", "C", "D"].iter().enumerate().map(|(i, n)| with_id(n, i as i64)).collect(), 0);
        let b = q.key_at(1);
        // The playlist is now D C N1 B N2 (A gone): keep the queue's order, new ones next.
        let items = vec![with_id("D", 3), with_id("C", 2), with_id("N1", 10), with_id("B", 1), with_id("N2", 11)];
        let summary = q.update(items, b, UpdateOptions { new_items: NewItems::Next, keep_queue_order: true, ..Default::default() });
        assert_eq!(summary, UpdateSummary { added: 2, removed: 1 });
        assert_eq!(titles(&q), ["B", "N1", "N2", "C", "D"]);

        // Keep the queue's order, new ones in place: after the song before them in the playlist.
        let mut q = Queue::new(Rng::new(3));
        q.set(["A", "B", "C"].iter().enumerate().map(|(i, n)| with_id(n, i as i64)).collect(), 0);
        let items = vec![with_id("N0", 9), with_id("C", 2), with_id("N1", 10), with_id("A", 0), with_id("B", 1)];
        q.update(items, q.key_at(0), UpdateOptions { keep_queue_order: true, ..Default::default() });
        assert_eq!(titles(&q), ["N0", "A", "B", "C", "N1"]);

        // With shuffle, "next" puts them right after the playing song in play order.
        let mut q = Queue::new(Rng::new(6));
        q.set((0..5).map(|i| with_id(&format!("T{i}"), i)).collect(), 0);
        q.set_shuffle(true, q.key_at(0));
        let current = q.key_at(q.order()[2]);
        let items = (0..5).map(|i| with_id(&format!("T{i}"), i)).chain([with_id("N1", 10), with_id("N2", 11)]).collect();
        q.update(items, current, UpdateOptions { new_items: NewItems::Next, ..Default::default() });
        let names = titles(&q);
        assert_eq!(names[3..5], ["N1", "N2"]);
    }

    #[test]
    fn update_matches_the_same_song_twice_in_order() {
        let mut q = Queue::new(Rng::new(3));
        q.set(vec![with_id("A", 1), with_id("B", 2), with_id("A", 1)], 0);
        let keys: Vec<u64> = q.entries().iter().map(|e| e.key).collect();
        q.update(vec![with_id("A", 1), with_id("A", 1)], None, UpdateOptions::default());
        assert_eq!(q.entries().iter().map(|e| e.key).collect::<Vec<_>>(), vec![keys[0], keys[2]]);
    }

    #[test]
    fn update_with_shuffle_keeps_the_order_and_puts_new_entries_after_the_current() {
        let mut q = Queue::new(Rng::new(5));
        q.set((0..10).map(|i| with_id(&format!("T{i}"), i)).collect(), 0);
        q.set_shuffle(true, q.key_at(0));
        let current = q.key_at(q.order()[4]).unwrap();
        let before: Vec<u64> = q.order().iter().map(|&i| q.entries()[i].key).collect();
        let items: Vec<Item> = (0..10).filter(|i| *i != 7).map(|i| with_id(&format!("T{i}"), i)).chain([with_id("N", 99)]).collect();
        q.update(items, Some(current), UpdateOptions::default());
        let after: Vec<u64> = q.order().iter().map(|&i| q.entries()[i].key).collect();
        let removed: Vec<u64> = before.iter().copied().filter(|k| !after.contains(k)).collect();
        assert_eq!(removed.len(), 1); // T7
        let kept: Vec<u64> = after.iter().copied().filter(|k| before.contains(k)).collect();
        assert_eq!(kept, before.iter().copied().filter(|k| after.contains(k)).collect::<Vec<_>>());
        let new_at = after.iter().position(|k| !before.contains(k)).unwrap();
        assert!(new_at > after.iter().position(|k| *k == current).unwrap());
        assert_eq!(after.len(), 10);
    }

    #[test]
    fn moving_without_shuffle_changes_what_plays_next() {
        let mut q = queue(&["A", "B", "C", "D"]);
        let [a, d] = [q.key_at(0).unwrap(), q.key_at(3).unwrap()];
        assert!(q.move_entry(3, 1)); // A D B C
        assert_eq!(titles(&q), ["A", "D", "B", "C"]);
        assert_eq!(q.peek_next(a, true), Some(d));
        assert!(!q.move_entry(0, 4));
    }

    #[test]
    fn with_shuffle_edits_keep_keys_and_play_order() {
        let mut q = queue(&["A", "B", "C", "D"]);
        q.shuffle = true; // the list order as the "shuffled" order, to keep the test readable
        let c = q.key_at(2).unwrap();
        assert!(q.move_entry(2, 0)); // C A B D
        assert_eq!(q.index_of(c), Some(0));
        assert_eq!(titles(&q), ["A", "B", "C", "D"]); // play order unchanged
        let a = q.key_at(1).unwrap();
        assert_eq!(q.remove(1), Some(a)); // C B D
        assert_eq!(titles(&q), ["B", "C", "D"]);
        assert_eq!(q.index_of(c), Some(0));
        assert_eq!(q.peek_next(c, true), q.key_at(2)); // C then D
    }
}
