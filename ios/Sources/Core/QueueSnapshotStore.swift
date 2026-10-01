import Foundation

private let queueDefaultsKey = "tauri_native_audio_queue_v1"
private let queueIndexDefaultsKey = "tauri_native_audio_queue_index_v1"
private let queuePositionDefaultsKey = "tauri_native_audio_queue_position_v1"
private let queuePositionSaveThrottleSeconds: TimeInterval = 1.0

/// The last queue, saved so the app can resume it with `restoreLastQueue`. The queue itself is
/// written when it changes; the current index and position are saved about once a second.
/// Mirrors `QueueSnapshotStore.kt` on Android.
final class QueueSnapshotStore {
  struct SavedItem: Codable {
    let src: String
    let id: Int64?
    let title: String?
    let artist: String?
    let artworkUrl: String?
    let added: Bool?
  }

  private struct SavedQueue: Codable {
    let items: [SavedItem]
    let shuffle: Bool
    let playOrder: [Int]
    let repeatMode: String
    let sourceId: String?
  }

  struct Snapshot {
    let items: [QueueEntry]
    let index: Int
    let position: Double
    let shuffle: Bool
    /// Nil when there's no valid saved order (it's rebuilt on restore).
    let playOrder: [Int]?
    let repeatMode: RepeatMode
    let sourceId: String?
  }

  private let defaults: UserDefaults
  private var lastPositionSavedAt = Date.distantPast

  init(defaults: UserDefaults = .standard) {
    self.defaults = defaults
  }

  func saveQueue(_ queue: [QueueEntry], shuffle: Bool, playOrder: [Int], repeatMode: RepeatMode, sourceId: String?) {
    guard !queue.isEmpty else {
      defaults.removeObject(forKey: queueDefaultsKey)
      return
    }
    let saved = SavedQueue(
      items: queue.map {
        SavedItem(
          src: $0.src,
          id: $0.id,
          title: $0.metadata.title,
          artist: $0.metadata.artist,
          artworkUrl: $0.metadata.artworkURL,
          added: $0.addedToQueue
        )
      },
      shuffle: shuffle,
      playOrder: playOrder,
      repeatMode: repeatMode.rawValue,
      sourceId: sourceId
    )
    if let data = try? JSONEncoder().encode(saved) {
      defaults.set(data, forKey: queueDefaultsKey)
    }
  }

  func savePosition(index: Int, position: Double, force: Bool, now: Date = Date()) {
    guard index >= 0, position.isFinite else {
      return
    }
    if !force, now.timeIntervalSince(lastPositionSavedAt) < queuePositionSaveThrottleSeconds {
      return
    }
    lastPositionSavedAt = now
    defaults.set(index, forKey: queueIndexDefaultsKey)
    defaults.set(max(0, position), forKey: queuePositionDefaultsKey)
  }

  func load() -> Snapshot? {
    guard
      let data = defaults.data(forKey: queueDefaultsKey),
      let saved = try? JSONDecoder().decode(SavedQueue.self, from: data),
      !saved.items.isEmpty
    else {
      return nil
    }
    let items = saved.items.map {
      QueueEntry(
        src: $0.src,
        id: $0.id,
        metadata: PlaybackMetadata(title: $0.title, artist: $0.artist, artworkURL: $0.artworkUrl),
        addedToQueue: $0.added ?? false
      )
    }
    let validOrder = saved.playOrder.sorted() == Array(items.indices) ? saved.playOrder : nil
    let index = min(max(defaults.integer(forKey: queueIndexDefaultsKey), 0), items.count - 1)
    return Snapshot(
      items: items,
      index: index,
      position: max(0, defaults.double(forKey: queuePositionDefaultsKey)),
      shuffle: saved.shuffle,
      playOrder: validOrder,
      repeatMode: RepeatMode(rawValue: saved.repeatMode) ?? .off,
      sourceId: saved.sourceId
    )
  }
}
