import Foundation

private let itemProgressDefaultsKey = "tauri_native_audio_item_progress_v1"
private let trackProgressDefaultsKey = "tauri_native_audio_track_progress_v1"
/// From here on an item counts as played (e.g. skipped credits at the end of an episode).
private let playedAt = 0.98

struct ItemProgressEntry: Codable, Sendable {
  let itemId: Int64
  let progress: Double
  let positionMs: Int64
  let updatedAtMs: Int64
}

struct ItemProgressPayload: Encodable, Sendable {
  let entries: [ItemProgressEntry]
}

/// How far each item was played (setOptions `trackProgress`). Mirrors `ItemProgress.kt` on Android.
final class ItemProgressStore {
  private let defaults: UserDefaults

  init(defaults: UserDefaults = .standard) {
    self.defaults = defaults
  }

  var trackProgress: Bool {
    get { defaults.object(forKey: trackProgressDefaultsKey) as? Bool ?? false }
    set { defaults.set(newValue, forKey: trackProgressDefaultsKey) }
  }

  func list(itemIds: [Int64]?) -> [ItemProgressEntry] {
    let all = read()
    guard let itemIds else {
      return Array(all.values)
    }
    return itemIds.compactMap { all[String($0)] }
  }

  /// Records playback of `itemId` at `position` of `duration` (seconds); `completed` counts as played.
  func record(itemId: Int64, position: Double, duration: Double, completed: Bool, now: Date = Date()) {
    guard trackProgress, completed || (duration.isFinite && duration > 0) else {
      return
    }
    let safePosition = position.isFinite ? max(0, position) : 0
    var progress = completed ? 1.0 : min(max(safePosition / duration, 0), 1)
    if progress >= playedAt {
      progress = 1
    }
    var all = read()
    all[String(itemId)] = ItemProgressEntry(
      itemId: itemId,
      progress: progress,
      positionMs: Int64((completed && duration.isFinite ? max(duration, safePosition) : safePosition) * 1000),
      updatedAtMs: Int64(now.timeIntervalSince1970 * 1000)
    )
    write(all)
  }

  /// `merge` keeps whichever version of each item is newer; otherwise replaces. Returns the result.
  func set(_ entries: [ItemProgressEntry], merge: Bool) -> [ItemProgressEntry] {
    var all = read()
    for entry in entries {
      let key = String(entry.itemId)
      if !merge || (all[key].map { entry.updatedAtMs > $0.updatedAtMs } ?? true) {
        all[key] = entry
      }
    }
    write(all)
    return entries.compactMap { all[String($0.itemId)] }
  }

  private func read() -> [String: ItemProgressEntry] {
    guard
      let data = defaults.data(forKey: itemProgressDefaultsKey),
      let all = try? JSONDecoder().decode([String: ItemProgressEntry].self, from: data)
    else {
      return [:]
    }
    return all
  }

  private func write(_ all: [String: ItemProgressEntry]) {
    if let data = try? JSONEncoder().encode(all) {
      defaults.set(data, forKey: itemProgressDefaultsKey)
    }
  }
}
