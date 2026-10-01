import Foundation

private let playbackEventsDefaultsKey = "tauri_native_audio_playback_events_v1"
private let maxLoggedPlaybackEvents = 500

/// A playback event as the app receives it. Mirrors `PlaybackEvents.kt` on Android.
struct PlaybackEventPayload: Codable, Sendable {
  let id: String
  /// `start`, `complete` or `skip`.
  let type: String
  let itemId: Int64
  let positionMs: Int64
  let atMs: Int64
}

struct PlaybackEventsPayload: Encodable, Sendable {
  let events: [PlaybackEventPayload]
}

/// A saved log of what played, kept until the app acknowledges it, so it can keep e.g. a recently
/// played list or play counts up to date. Only the newest entries are kept.
final class PlaybackEventLog {
  private let defaults: UserDefaults

  init(defaults: UserDefaults = .standard) {
    self.defaults = defaults
  }

  func record(type: String, itemId: Int64, position: Double, now: Date = Date()) -> PlaybackEventPayload {
    let event = PlaybackEventPayload(
      id: UUID().uuidString,
      type: type,
      itemId: itemId,
      positionMs: Int64(max(0, position.isFinite ? position : 0) * 1000.0),
      atMs: Int64(now.timeIntervalSince1970 * 1000.0)
    )
    var events = pending()
    events.append(event)
    write(Array(events.suffix(maxLoggedPlaybackEvents)))
    return event
  }

  /// Events not acknowledged yet, oldest first.
  func pending() -> [PlaybackEventPayload] {
    guard
      let data = defaults.data(forKey: playbackEventsDefaultsKey),
      let events = try? JSONDecoder().decode([PlaybackEventPayload].self, from: data)
    else {
      return []
    }
    return events
  }

  func acknowledge(ids: Set<String>) {
    write(pending().filter { !ids.contains($0.id) })
  }

  private func write(_ events: [PlaybackEventPayload]) {
    if let data = try? JSONEncoder().encode(events) {
      defaults.set(data, forKey: playbackEventsDefaultsKey)
    }
  }
}
