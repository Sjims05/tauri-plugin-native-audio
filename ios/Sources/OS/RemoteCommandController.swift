import Foundation
import MediaPlayer

enum RemoteCommandEvent: Sendable {
  case play
  case pause
  case toggle
  case seek(position: Double)
  case seekDelta(delta: Double)
  case nextTrack
  case previousTrack
}

final class RemoteCommandController {
  private var remoteCommandTargets: [(MPRemoteCommand, Any)] = []
  private var eventHandler: ((RemoteCommandEvent) -> Void)?
  /// 0 = next/previous track buttons; > 0 = skip forward/backward buttons that seek this many seconds.
  private var skipIntervalSeconds = 0.0
  private var hasNextTrack = false
  private var hasPreviousTrack = false

  deinit {
    unregister()
  }

  func registerIfNeeded(eventHandler: @escaping (RemoteCommandEvent) -> Void) {
    onMain {
      self.eventHandler = eventHandler
      if !remoteCommandTargets.isEmpty {
        return
      }

      let center = MPRemoteCommandCenter.shared()
      center.playCommand.isEnabled = true
      center.pauseCommand.isEnabled = true
      center.togglePlayPauseCommand.isEnabled = true
      center.changePlaybackPositionCommand.isEnabled = true

      let playTarget = center.playCommand.addTarget { [weak self] _ in
        self?.eventHandler?(.play)
        return .success
      }
      remoteCommandTargets.append((center.playCommand, playTarget))

      let pauseTarget = center.pauseCommand.addTarget { [weak self] _ in
        self?.eventHandler?(.pause)
        return .success
      }
      remoteCommandTargets.append((center.pauseCommand, pauseTarget))

      let toggleTarget = center.togglePlayPauseCommand.addTarget { [weak self] _ in
        self?.eventHandler?(.toggle)
        return .success
      }
      remoteCommandTargets.append((center.togglePlayPauseCommand, toggleTarget))

      let changePositionTarget = center.changePlaybackPositionCommand.addTarget { [weak self] event in
        guard let seekEvent = event as? MPChangePlaybackPositionCommandEvent else {
          return .commandFailed
        }
        self?.eventHandler?(.seek(position: seekEvent.positionTime))
        return .success
      }
      remoteCommandTargets.append((center.changePlaybackPositionCommand, changePositionTarget))

      let nextTrackTarget = center.nextTrackCommand.addTarget { [weak self] _ in
        self?.eventHandler?(.nextTrack)
        return .success
      }
      remoteCommandTargets.append((center.nextTrackCommand, nextTrackTarget))

      let previousTrackTarget = center.previousTrackCommand.addTarget { [weak self] _ in
        self?.eventHandler?(.previousTrack)
        return .success
      }
      remoteCommandTargets.append((center.previousTrackCommand, previousTrackTarget))

      let skipForwardTarget = center.skipForwardCommand.addTarget { [weak self] _ in
        guard let self else { return .commandFailed }
        self.eventHandler?(.seekDelta(delta: self.skipIntervalSeconds))
        return .success
      }
      remoteCommandTargets.append((center.skipForwardCommand, skipForwardTarget))

      let skipBackwardTarget = center.skipBackwardCommand.addTarget { [weak self] _ in
        guard let self else { return .commandFailed }
        self.eventHandler?(.seekDelta(delta: -self.skipIntervalSeconds))
        return .success
      }
      remoteCommandTargets.append((center.skipBackwardCommand, skipBackwardTarget))

      applyNavigationCommands()
    }
  }

  func setTrackCommandsEnabled(hasNext: Bool, hasPrevious: Bool) {
    onMain {
      hasNextTrack = hasNext
      hasPreviousTrack = hasPrevious
      applyNavigationCommands()
    }
  }

  func setSkipInterval(seconds: Double) {
    onMain {
      skipIntervalSeconds = seconds
      applyNavigationCommands()
    }
  }

  /// Skip-interval and track commands share the same lock screen slots, so only one pair is enabled.
  private func applyNavigationCommands() {
    guard !remoteCommandTargets.isEmpty else {
      return
    }
    let center = MPRemoteCommandCenter.shared()
    let useSkipInterval = skipIntervalSeconds > 0
    center.skipForwardCommand.isEnabled = useSkipInterval
    center.skipBackwardCommand.isEnabled = useSkipInterval
    if useSkipInterval {
      center.skipForwardCommand.preferredIntervals = [NSNumber(value: skipIntervalSeconds)]
      center.skipBackwardCommand.preferredIntervals = [NSNumber(value: skipIntervalSeconds)]
    }
    center.nextTrackCommand.isEnabled = !useSkipInterval && hasNextTrack
    center.previousTrackCommand.isEnabled = !useSkipInterval && hasPreviousTrack
  }

  func unregister() {
    onMain {
      let center = MPRemoteCommandCenter.shared()

      for (command, target) in remoteCommandTargets {
        command.removeTarget(target)
      }
      remoteCommandTargets.removeAll()
      eventHandler = nil

      center.playCommand.isEnabled = false
      center.pauseCommand.isEnabled = false
      center.togglePlayPauseCommand.isEnabled = false
      center.changePlaybackPositionCommand.isEnabled = false
      center.nextTrackCommand.isEnabled = false
      center.previousTrackCommand.isEnabled = false
      center.skipForwardCommand.isEnabled = false
      center.skipBackwardCommand.isEnabled = false
    }
  }

  private func onMain<T>(_ block: () -> T) -> T {
    if Thread.isMainThread {
      return block()
    }
    return DispatchQueue.main.sync(execute: block)
  }
}
