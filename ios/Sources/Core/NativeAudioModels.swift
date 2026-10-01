import Foundation

let nativeAudioStateEvent = "native_audio_state"
/// `previous` restarts the current track instead of going back when playback is past this point.
let previousRestartThresholdSeconds = 3.0
let checkpointDefaultsKeyV1 = "tauri_native_audio_progress_checkpoint_v1"

struct NativeAudioState: Encodable, Sendable {
  let status: String
  let currentTime: Double
  let duration: Double
  let isPlaying: Bool
  let buffering: Bool
  let rate: Double
  let queueIndex: Int
  let queueLength: Int
  let currentId: Int64?
  let shuffle: Bool
  let repeatMode: String
  /// When a running sleep timer pauses playback (epoch ms), or nil.
  let sleepTimerEndsAtMs: Int64?
  let sleepTimerEndOfTrack: Bool
  let error: String?
}

struct SetSleepTimerArgs: Decodable, Sendable {
  let minutes: Double?
  let endOfTrack: Bool?
  let fadeOutSeconds: Double?
}

struct ItemIdsArgs: Decodable, Sendable {
  let itemIds: [Int64]?
}

struct SetItemProgressArgs: Decodable, Sendable {
  let entries: [ItemProgressEntry]?
  let merge: Bool?
}

enum RepeatMode: String, Sendable {
  case off
  case all
  case one
}

struct NativeAudioProgressCheckpoint: Codable, Sendable {
  let id: Int64
  let currentTime: Double
  let updatedAtMs: Int64
  let status: String?
}

struct SetSourceArgs: Decodable, Sendable {
  let src: String
  let id: Int64?
  let title: String?
  let artist: String?
  let artworkUrl: String?
}

struct SetQueueArgs: Decodable, Sendable {
  let items: [SetSourceArgs]
  let startIndex: Int?
  let startPosition: Double?
  let sourceId: String?
}

struct SetTrackedListsArgs: Decodable, Sendable {
  let lists: [TrackedListConfig]
}

struct TrackedListArgs: Decodable, Sendable {
  let id: String?
}

struct SetTrackedListArgs: Decodable, Sendable {
  let id: String?
  let entries: [TrackedListEntry]?
  let merge: Bool?
}

struct AddToQueueArgs: Decodable, Sendable {
  let items: [SetSourceArgs]
  let playNext: Bool?
}

struct MoveInQueueArgs: Decodable, Sendable {
  let from: Int?
  let to: Int?
}

/// A queue entry as getQueue reports it: the same shape as a setQueue item.
struct QueueItemPayload: Encodable, Sendable {
  let src: String
  let id: Int64?
  let title: String?
  let artist: String?
  let artworkUrl: String?
}

struct QueuePayload: Encodable, Sendable {
  let items: [QueueItemPayload]
  let currentIndex: Int
  /// Queue indices in the order they play (the shuffle order when shuffle is on).
  let playOrder: [Int]
}

struct SkipToArgs: Decodable, Sendable {
  let index: Int?
}

struct SetShuffleArgs: Decodable, Sendable {
  let enabled: Bool?
}

struct SetRepeatModeArgs: Decodable, Sendable {
  let mode: String?
}

struct SetSkipIntervalArgs: Decodable, Sendable {
  let seconds: Double?
}

struct SeekToArgs: Decodable, Sendable {
  let position: Double?
}

struct SetRateArgs: Decodable, Sendable {
  let rate: Double?
}

enum NativeAudioRuntimeError: LocalizedError {
  case invalidSource
  case invalidRate
  case invalidSkipInterval
  case emptyQueue
  case indexOutOfRange

  var errorDescription: String? {
    switch self {
    case .invalidSource:
      return "invalid source"
    case .invalidRate:
      return "rate must be > 0"
    case .invalidSkipInterval:
      return "seconds must be >= 0"
    case .emptyQueue:
      return "items must not be empty"
    case .indexOutOfRange:
      return "index out of range"
    }
  }
}

struct PlaybackMetadata: Sendable {
  let title: String?
  let artist: String?
  let artworkURL: String?
}

struct QueueEntry: Sendable {
  let src: String
  let id: Int64?
  let metadata: PlaybackMetadata
  /// Added with addToQueue; dropped when the queue repeats unless repeatAddedTracks is on.
  var addedToQueue = false
}

struct AcknowledgeIdsArgs: Decodable, Sendable {
  let ids: [String]?
}

struct SetOptionsArgs: Decodable, Sendable {
  let resumeLastQueue: Bool?
  let repeatAddedTracks: Bool?
  /// Android only: iOS keeps the lock screen controls while paused on its own.
  let pausedKeepAliveMinutes: Double?
  /// Android only (Android Auto).
  let keepAliveWhileCarConnected: Bool?
  let trackProgress: Bool?
}

struct RuntimeSnapshot: Sendable {
  let sourceRevision: Int64
  let seekRevision: Int64
  let state: NativeAudioState
}
