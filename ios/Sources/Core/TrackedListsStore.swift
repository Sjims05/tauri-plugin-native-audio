import Foundation

private let configsDefaultsKey = "tauri_native_audio_tracked_list_configs_v1"
private let listsDefaultsKey = "tauri_native_audio_tracked_lists_v1"
private let changesDefaultsKey = "tauri_native_audio_tracked_list_changes_v1"
private let defaultTrackedListLimit = 10

struct TrackedListConfig: Codable, Sendable {
  let id: String
  /// "folder": the folder (playlist, album) a queue was started from. "item": tracks.
  let track: String
  let limit: Int?
  /// How long something has to play before it counts; 0 = as soon as it starts.
  let countAfterSeconds: Double?
}

/// A tracked list entry: a folder id (string) or an item id (number).
enum TrackedListValue: Codable, Sendable, Hashable {
  case item(Int64)
  case folder(String)

  init(from decoder: Decoder) throws {
    let container = try decoder.singleValueContainer()
    if let itemId = try? container.decode(Int64.self) {
      self = .item(itemId)
    } else {
      self = .folder(try container.decode(String.self))
    }
  }

  func encode(to encoder: Encoder) throws {
    var container = encoder.singleValueContainer()
    switch self {
    case let .item(itemId):
      try container.encode(itemId)
    case let .folder(folderId):
      try container.encode(folderId)
    }
  }
}

/// A tracked list entry with when it was last played, so versions from elsewhere can be merged.
struct TrackedListEntry: Codable, Sendable, Equatable {
  let id: TrackedListValue
  let playedAtMs: Int64
}

struct TrackedListChange: Codable, Sendable {
  let id: String
  let listId: String
  let entries: [TrackedListEntry]
  /// The entries' ids, as a shortcut.
  let ids: [TrackedListValue]
  let changedAtMs: Int64
}

struct TrackedListChangesPayload: Encodable, Sendable {
  let changes: [TrackedListChange]
}

struct TrackedListEntriesPayload: Encodable, Sendable {
  let entries: [TrackedListEntry]
}

enum TrackedListsError: LocalizedError {
  case invalid(String)

  var errorDescription: String? {
    switch self {
    case let .invalid(message):
      return message
    }
  }
}

/// Lists the app declares with `setTrackedLists`, kept up to date natively: most recent first, no
/// duplicates, capped at a limit. Changes wait for the app to acknowledge them (only the latest
/// per list). Mirrors `TrackedLists.kt` on Android.
final class TrackedListsStore {
  private let defaults: UserDefaults
  private var cachedConfigs: [TrackedListConfig]?

  init(defaults: UserDefaults = .standard) {
    self.defaults = defaults
  }

  func setConfigs(_ configs: [TrackedListConfig]) throws {
    var seen = Set<String>()
    for config in configs {
      guard !config.id.isEmpty else { throw TrackedListsError.invalid("every list requires id") }
      guard seen.insert(config.id).inserted else { throw TrackedListsError.invalid("duplicate list id \(config.id)") }
      guard config.track == "folder" || config.track == "item" else {
        throw TrackedListsError.invalid("list \(config.id): track must be folder or item")
      }
      guard (config.limit ?? defaultTrackedListLimit) > 0 else { throw TrackedListsError.invalid("list \(config.id): limit must be > 0") }
      guard (config.countAfterSeconds ?? 0) >= 0 else {
        throw TrackedListsError.invalid("list \(config.id): countAfterSeconds must be >= 0")
      }
    }

    let lists = readLists()
    var kept: [String: [TrackedListEntry]] = [:]
    for config in configs {
      kept[config.id] = Array((lists[config.id] ?? []).prefix(limit(of: config)))
    }
    let changes = readChanges().filter { change in configs.contains { $0.id == change.key } }
    write(configs, forKey: configsDefaultsKey)
    write(kept, forKey: listsDefaultsKey)
    write(changes, forKey: changesDefaultsKey)
    cachedConfigs = configs
  }

  func configs() -> [TrackedListConfig] {
    if let cachedConfigs {
      return cachedConfigs
    }
    let loaded: [TrackedListConfig] = read(forKey: configsDefaultsKey) ?? []
    cachedConfigs = loaded
    return loaded
  }

  /// The list's entries, newest first.
  func entries(listId: String) -> [TrackedListEntry] {
    readLists()[listId] ?? []
  }

  /// Sets a list from the app: `merge` combines with the current entries (by id, newest time wins),
  /// otherwise replaces them. Not reported back as a change. Returns the result.
  func set(listId: String, entries incoming: [TrackedListEntry], merge: Bool) throws -> [TrackedListEntry] {
    guard let config = configs().first(where: { $0.id == listId }) else {
      throw TrackedListsError.invalid("unknown list \(listId)")
    }
    var lists = readLists()
    let base = merge ? (lists[listId] ?? []) : []
    let result = Self.combine(base + incoming, limit: limit(of: config))
    lists[listId] = result
    write(lists, forKey: listsDefaultsKey)
    return result
  }

  /// Records that `value` was played now. Returns the change to report, or nil if nothing changed.
  func record(config: TrackedListConfig, value: TrackedListValue, now: Date = Date()) -> TrackedListChange? {
    var lists = readLists()
    let current = lists[config.id] ?? []
    let nowMs = Int64(now.timeIntervalSince1970 * 1000.0)
    let updated = Self.combine(current + [TrackedListEntry(id: value, playedAtMs: nowMs)], limit: limit(of: config))
    guard updated != current else {
      return nil
    }
    lists[config.id] = updated
    let change = TrackedListChange(
      id: UUID().uuidString,
      listId: config.id,
      entries: updated,
      ids: updated.map(\.id),
      changedAtMs: nowMs
    )
    var changes = readChanges()
    changes[config.id] = change
    write(lists, forKey: listsDefaultsKey)
    write(changes, forKey: changesDefaultsKey)
    return change
  }

  /// The latest unacknowledged change of each list, oldest first.
  func pendingChanges() -> [TrackedListChange] {
    readChanges().values.sorted { $0.changedAtMs < $1.changedAtMs }
  }

  func acknowledge(ids: Set<String>) {
    write(readChanges().filter { !ids.contains($0.value.id) }, forKey: changesDefaultsKey)
  }

  /// One entry per id with its newest time, newest first, at most `limit`.
  static func combine(_ entries: [TrackedListEntry], limit: Int) -> [TrackedListEntry] {
    var newest: [TrackedListValue: TrackedListEntry] = [:]
    var order: [TrackedListValue] = []
    for entry in entries {
      if let existing = newest[entry.id] {
        if entry.playedAtMs > existing.playedAtMs {
          newest[entry.id] = entry
        }
      } else {
        newest[entry.id] = entry
        order.append(entry.id)
      }
    }
    // Stable: equal times keep their first-seen order.
    let combined = order.enumerated()
      .map { (index: $0.offset, entry: newest[$0.element]!) }
      .sorted { $0.entry.playedAtMs != $1.entry.playedAtMs ? $0.entry.playedAtMs > $1.entry.playedAtMs : $0.index < $1.index }
      .map(\.entry)
    return Array(combined.prefix(limit))
  }

  private func limit(of config: TrackedListConfig) -> Int {
    config.limit ?? defaultTrackedListLimit
  }

  private func readLists() -> [String: [TrackedListEntry]] {
    if let lists: [String: [TrackedListEntry]] = read(forKey: listsDefaultsKey) {
      return lists
    }
    // Saved before entries had times.
    let old: [String: [TrackedListValue]] = read(forKey: listsDefaultsKey) ?? [:]
    return old.mapValues { ids in ids.map { TrackedListEntry(id: $0, playedAtMs: 0) } }
  }

  private func readChanges() -> [String: TrackedListChange] {
    read(forKey: changesDefaultsKey) ?? [:]
  }

  private func read<T: Decodable>(forKey key: String) -> T? {
    guard let data = defaults.data(forKey: key) else {
      return nil
    }
    return try? JSONDecoder().decode(T.self, from: data)
  }

  private func write<T: Encodable>(_ value: T, forKey key: String) {
    if let data = try? JSONEncoder().encode(value) {
      defaults.set(data, forKey: key)
    }
  }
}
