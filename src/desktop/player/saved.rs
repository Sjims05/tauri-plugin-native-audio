//! The saved logs and lists: playback events, tracked lists, item progress, the checkpoint.

use super::*;

impl DesktopAudio {
    pub fn playback_events(&self) -> Value {
        json!({ "events": self.persist.lock().unwrap().pending_events() })
    }

    pub fn acknowledge_playback_events(&self, ids: &[String]) {
        self.persist.lock().unwrap().acknowledge_events(ids);
    }

    pub fn set_tracked_lists(&self, lists: Vec<ListConfig>) -> Result<()> {
        self.persist.lock().unwrap().set_list_configs(lists)
    }

    pub fn tracked_list(&self, id: &str) -> Value {
        json!({ "entries": self.persist.lock().unwrap().list_entries(id) })
    }

    pub fn set_tracked_list(&self, id: &str, entries: Vec<ListEntry>, merge: bool) -> Result<Value> {
        Ok(json!({ "entries": self.persist.lock().unwrap().set_list(id, entries, merge)? }))
    }

    pub fn tracked_list_changes(&self) -> Value {
        json!({ "changes": self.persist.lock().unwrap().pending_list_changes() })
    }

    pub fn acknowledge_tracked_list_changes(&self, ids: &[String]) {
        self.persist.lock().unwrap().acknowledge_list_changes(ids);
    }

    pub fn item_progress(&self, item_ids: Option<Vec<i64>>) -> Value {
        json!({ "entries": self.persist.lock().unwrap().progress(item_ids.as_deref()) })
    }

    pub fn set_item_progress(&self, entries: Vec<ProgressEntry>, merge: bool) -> Result<Value> {
        let mut parsed = Vec::with_capacity(entries.len());
        for (i, e) in entries.into_iter().enumerate() {
            if !e.progress.is_finite() || !(0.0..=1.0).contains(&e.progress) {
                return Err(format!("entries[{i}]: progress must be between 0 and 1"));
            }
            let updated_at_ms = e.updated_at_ms.unwrap_or_else(crate::desktop::persist::now_ms);
            parsed.push((e.item_id, Progress { progress: e.progress, position_ms: e.position_ms.unwrap_or(0).max(0), updated_at_ms }));
        }
        Ok(json!({ "entries": self.persist.lock().unwrap().set_progress(parsed, merge) }))
    }

    pub fn progress_checkpoint(&self) -> Option<Value> {
        self.persist.lock().unwrap().checkpoint()
    }

    pub fn clear_progress_checkpoint(&self) {
        self.persist.lock().unwrap().clear_checkpoint();
    }
}

/// An item's progress as setItemProgress sends it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEntry {
    pub item_id: i64,
    pub progress: f64,
    pub position_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
}
