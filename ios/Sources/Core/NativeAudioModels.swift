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
  let error: String?
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
}

struct SkipToArgs: Decodable, Sendable {
  let index: Int?
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
}

struct RuntimeSnapshot: Sendable {
  let sourceRevision: Int64
  let seekRevision: Int64
  let state: NativeAudioState
}
