import Foundation
import Tauri

class NativeAudioPlugin: Plugin, NativeAudioEventEmitter {
  private let runtime = PlaybackRuntimeActor.shared

  override init() {
    super.init()
    Task { @MainActor in
      await runtime.attachEmitter(self)
    }
  }

  deinit {
    let runtime = runtime
    Task {
      await runtime.attachEmitter(nil)
    }
  }

  func emitNativeAudioState(_ state: NativeAudioState) {
    try? trigger(nativeAudioStateEvent, data: state)
  }

  func emitPlaybackEvent(_ event: PlaybackEventPayload) {
    try? trigger("native_audio_playback_event", data: event)
  }

  func emitTrackedListChange(_ change: TrackedListChange) {
    try? trigger("native_audio_tracked_list", data: change)
  }

  @objc public func setSleepTimer(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        let args = try invoke.parseArgs(SetSleepTimerArgs.self)
        let endOfTrack = args.endOfTrack ?? false
        if !endOfTrack {
          guard let minutes = args.minutes, minutes.isFinite, minutes > 0 else {
            invoke.reject("minutes must be > 0, or endOfTrack true")
            return
          }
        }
        let fadeOutSeconds = args.fadeOutSeconds ?? 10
        guard fadeOutSeconds.isFinite, fadeOutSeconds >= 0 else {
          invoke.reject("fadeOutSeconds must be >= 0")
          return
        }
        invoke.resolve(await runtime.setSleepTimer(minutes: args.minutes, endOfTrack: endOfTrack, fadeOutSeconds: fadeOutSeconds))
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func cancelSleepTimer(_ invoke: Invoke) {
    Task { @MainActor in
      invoke.resolve(await runtime.cancelSleepTimer())
    }
  }

  @objc public func getItemProgress(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        let args = try invoke.parseArgs(ItemIdsArgs.self)
        invoke.resolve(ItemProgressPayload(entries: await runtime.itemProgress(itemIds: args.itemIds)))
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func setItemProgress(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        let args = try invoke.parseArgs(SetItemProgressArgs.self)
        guard let entries = args.entries else {
          invoke.reject("entries is required")
          return
        }
        invoke.resolve(ItemProgressPayload(entries: await runtime.setItemProgress(entries, merge: args.merge ?? false)))
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func setTrackedLists(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        let args = try invoke.parseArgs(SetTrackedListsArgs.self)
        try await runtime.setTrackedLists(args.lists)
        invoke.resolve()
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func getTrackedList(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        let args = try invoke.parseArgs(TrackedListArgs.self)
        guard let id = args.id, !id.isEmpty else {
          invoke.reject("id is required")
          return
        }
        invoke.resolve(TrackedListEntriesPayload(entries: await runtime.trackedList(id: id)))
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func setTrackedList(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        let args = try invoke.parseArgs(SetTrackedListArgs.self)
        guard let id = args.id, !id.isEmpty, let entries = args.entries else {
          invoke.reject("id and entries are required")
          return
        }
        let result = try await runtime.setTrackedList(id: id, entries: entries, merge: args.merge ?? false)
        invoke.resolve(TrackedListEntriesPayload(entries: result))
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func getTrackedListChanges(_ invoke: Invoke) {
    Task { @MainActor in
      invoke.resolve(TrackedListChangesPayload(changes: await runtime.pendingTrackedListChanges()))
    }
  }

  @objc public func acknowledgeTrackedListChanges(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        let args = try invoke.parseArgs(AcknowledgeIdsArgs.self)
        await runtime.acknowledgeTrackedListChanges(ids: Set(args.ids ?? []))
        invoke.resolve()
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func getPlaybackEvents(_ invoke: Invoke) {
    Task { @MainActor in
      invoke.resolve(PlaybackEventsPayload(events: await runtime.pendingPlaybackEvents()))
    }
  }

  @objc public func acknowledgePlaybackEvents(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        let args = try invoke.parseArgs(AcknowledgeIdsArgs.self)
        await runtime.acknowledgePlaybackEvents(ids: Set(args.ids ?? []))
        invoke.resolve()
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func initialize(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        invoke.resolve(try await runtime.initialize())
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func setSource(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        let args = try invoke.parseArgs(SetSourceArgs.self)
        let src = args.src.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !src.isEmpty else {
          invoke.reject("src is required")
          return
        }

        invoke.resolve(
          try await runtime.setSource(
            src: src,
            id: args.id,
            title: args.title,
            artist: args.artist,
            artworkURL: args.artworkUrl
          )
        )
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func setQueue(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        let args = try invoke.parseArgs(SetQueueArgs.self)
        var items: [QueueEntry] = []
        for item in args.items {
          let src = item.src.trimmingCharacters(in: .whitespacesAndNewlines)
          guard !src.isEmpty else {
            invoke.reject("every item requires src")
            return
          }
          items.append(
            QueueEntry(
              src: src,
              id: item.id,
              metadata: PlaybackMetadata(title: item.title, artist: item.artist, artworkURL: item.artworkUrl)
            )
          )
        }

        invoke.resolve(
          try await runtime.setQueue(
            items: items,
            startIndex: args.startIndex ?? 0,
            startPosition: args.startPosition ?? 0.0,
            sourceId: args.sourceId
          )
        )
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func next(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        invoke.resolve(try await runtime.next())
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func previous(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        invoke.resolve(try await runtime.previous())
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func skipTo(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        let args = try invoke.parseArgs(SkipToArgs.self)
        guard let index = args.index else {
          invoke.reject("index is required")
          return
        }

        invoke.resolve(try await runtime.skipTo(index: index))
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func play(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        invoke.resolve(try await runtime.play())
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func pause(_ invoke: Invoke) {
    Task { @MainActor in
      invoke.resolve(await runtime.pause())
    }
  }

  @objc public func seekTo(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        let args = try invoke.parseArgs(SeekToArgs.self)
        guard let position = args.position, position.isFinite else {
          invoke.reject("position is required")
          return
        }

        invoke.resolve(await runtime.seekTo(position: position))
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func setRate(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        let args = try invoke.parseArgs(SetRateArgs.self)
        guard let rate = args.rate, rate.isFinite, rate > 0 else {
          invoke.reject("rate must be > 0")
          return
        }

        invoke.resolve(try await runtime.setRate(rate: rate))
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func setShuffle(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        let args = try invoke.parseArgs(SetShuffleArgs.self)
        guard let enabled = args.enabled else {
          invoke.reject("enabled is required")
          return
        }

        invoke.resolve(await runtime.setShuffle(enabled: enabled))
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func setRepeatMode(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        let args = try invoke.parseArgs(SetRepeatModeArgs.self)
        guard let mode = args.mode.flatMap(RepeatMode.init(rawValue:)) else {
          invoke.reject("mode must be off, all or one")
          return
        }

        invoke.resolve(await runtime.setRepeatMode(mode))
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func addToQueue(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        let args = try invoke.parseArgs(AddToQueueArgs.self)
        var items: [QueueEntry] = []
        for item in args.items {
          let src = item.src.trimmingCharacters(in: .whitespacesAndNewlines)
          guard !src.isEmpty else {
            invoke.reject("every item requires src")
            return
          }
          items.append(
            QueueEntry(
              src: src,
              id: item.id,
              metadata: PlaybackMetadata(title: item.title, artist: item.artist, artworkURL: item.artworkUrl)
            )
          )
        }

        invoke.resolve(try await runtime.addToQueue(items: items, playNext: args.playNext ?? false))
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func removeFromQueue(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        let args = try invoke.parseArgs(SkipToArgs.self)
        guard let index = args.index else {
          invoke.reject("index is required")
          return
        }

        invoke.resolve(try await runtime.removeFromQueue(index: index))
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func moveInQueue(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        let args = try invoke.parseArgs(MoveInQueueArgs.self)
        guard let from = args.from, let to = args.to else {
          invoke.reject("from and to are required")
          return
        }

        invoke.resolve(try await runtime.moveInQueue(from: from, to: to))
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func getQueue(_ invoke: Invoke) {
    Task { @MainActor in
      let queue = await runtime.getQueue()
      invoke.resolve(
        QueuePayload(
          items: queue.items.map {
            QueueItemPayload(src: $0.src, id: $0.id, title: $0.metadata.title, artist: $0.metadata.artist, artworkUrl: $0.metadata.artworkURL)
          },
          currentIndex: queue.currentIndex,
          playOrder: queue.playOrder
        )
      )
    }
  }

  @objc public func restoreLastQueue(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        if let state = try await runtime.restoreLastQueue() {
          invoke.resolve(state)
        } else {
          // Nothing saved: resolves with null.
          invoke.resolve()
        }
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func setOptions(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        let args = try invoke.parseArgs(SetOptionsArgs.self)
        // resumeLastQueue only affects Android's automatic resume; iOS resumes only via restoreLastQueue.
        await runtime.setOptions(repeatAddedTracks: args.repeatAddedTracks, trackProgress: args.trackProgress)
        invoke.resolve()
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  /// The setControls buttons are shown by Android Auto and Android's media controls. iOS has no
  /// place for custom buttons yet (that would be CarPlay), so these are accepted and do nothing.
  @objc public func setControls(_ invoke: Invoke) {
    invoke.resolve()
  }

  @objc public func setControlActive(_ invoke: Invoke) {
    invoke.resolve()
  }

  @objc public func getControlPresses(_ invoke: Invoke) {
    invoke.resolve(["presses": [Any]()])
  }

  @objc public func acknowledgeControlPresses(_ invoke: Invoke) {
    invoke.resolve()
  }

  /// Android Auto library. Accepted so the same JS works on both platforms; not used on iOS yet
  /// (CarPlay would use it).
  @objc public func setLibrary(_ invoke: Invoke) {
    invoke.resolve()
  }

  @objc public func setSkipInterval(_ invoke: Invoke) {
    Task { @MainActor in
      do {
        let args = try invoke.parseArgs(SetSkipIntervalArgs.self)
        guard let seconds = args.seconds, seconds.isFinite, seconds >= 0 else {
          invoke.reject("seconds must be >= 0")
          return
        }

        invoke.resolve(try await runtime.setSkipInterval(seconds: seconds))
      } catch {
        invoke.reject(error.localizedDescription)
      }
    }
  }

  @objc public func getState(_ invoke: Invoke) {
    Task { @MainActor in
      invoke.resolve(await runtime.getState())
    }
  }

  @objc public func getProgressCheckpoint(_ invoke: Invoke) {
    Task { @MainActor in
      invoke.resolve(await runtime.getProgressCheckpoint())
    }
  }

  @objc public func clearProgressCheckpoint(_ invoke: Invoke) {
    Task { @MainActor in
      await runtime.clearProgressCheckpoint()
      invoke.resolve()
    }
  }

  @objc public func dispose(_ invoke: Invoke) {
    Task { @MainActor in
      await runtime.dispose()
      invoke.resolve()
    }
  }
}

extension NativeAudioPlugin: @unchecked Sendable {}

@_cdecl("init_plugin_native_audio")
func initPlugin() -> Plugin {
  NativeAudioPlugin()
}
