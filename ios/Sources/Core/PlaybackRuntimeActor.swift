@preconcurrency import Foundation
import UIKit

private let seekCommitEpsilonSeconds = 0.02
private let repeatAddedTracksDefaultsKey = "tauri_native_audio_repeat_added_tracks_v1"
private let foregroundProgressEmitIntervalSeconds = 1.0 / 40.0
private let backgroundProgressEmitIntervalSeconds = 0.25

protocol NativeAudioEventEmitter: AnyObject, Sendable {
  func emitNativeAudioState(_ state: NativeAudioState)
  func emitPlaybackEvent(_ event: PlaybackEventPayload)
  func emitTrackedListChange(_ change: TrackedListChange)
}

actor PlaybackRuntimeActor {
  static let shared = PlaybackRuntimeActor()

  private let playerAdapter = PlayerAdapter()
  private let audioSessionController = AudioSessionController()
  private let nowPlayingController = NowPlayingController()
  private let remoteCommandController = RemoteCommandController()
  private let sourceResolver = SourceResolver()
  private let checkpointStore = CheckpointStore()
  private let queueSnapshotStore = QueueSnapshotStore()
  private let playbackEventLog = PlaybackEventLog()
  /// Whether the current track's `start` playback event was logged (it's logged once it plays).
  private var currentStartLogged = false
  private let trackedListsStore = TrackedListsStore()
  /// The playable folder (playlist, album) the queue was started from, for "folder" tracked lists.
  private var queueSourceId: String?
  /// Tracked lists the current play of the current track has already counted towards.
  private var countedLists = Set<String>()
  private let itemProgressStore = ItemProgressStore()
  private var lastItemProgressSavedAt = Date.distantPast
  // Sleep timer: a time (with a fade-out before it) or the end of the current track.
  private var sleepTimerTask: Task<Void, Never>?
  private var sleepTimerEndsAt: Date?
  private var sleepTimerEndOfTrack = false

  private weak var emitter: NativeAudioEventEmitter?

  private var machine = PlaybackStateMachine()
  private var isConfigured = false
  private var wasPlayingBeforeInterruption = false
  private var isAppInForeground = true

  private var queue: [QueueEntry] = []
  private var queueIndex = -1
  private var queueLoadRevision: Int64 = 0
  /// Queue indices in play order: shuffled when shuffle is on, 0..<queue.count otherwise.
  private var playOrder: [Int] = []
  // Kept across dispose() so they only have to be set once.
  private var shuffleEnabled = false
  private var repeatMode = RepeatMode.off
  private var repeatAddedTracks = UserDefaults.standard.object(forKey: repeatAddedTracksDefaultsKey) as? Bool ?? false

  private var lastEmittedState: NativeAudioState?
  private var lastProgressTickEmitAt = Date.distantPast
  private var appDidBecomeActiveObserver: NSObjectProtocol?
  private var appDidEnterBackgroundObserver: NSObjectProtocol?

  private enum EmitTrigger {
    case transition
    case progressTick
  }

  func attachEmitter(_ emitter: NativeAudioEventEmitter?) {
    self.emitter = emitter
  }

  func initialize() async throws -> NativeAudioState {
    ensureConfigured()
    try audioSessionController.configurePlaybackCategory()
    emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: true, refreshArtwork: false)
    return snapshot()
  }

  func setSource(
    src: String,
    id: Int64?,
    title: String?,
    artist: String?,
    artworkURL: String?
  ) async throws -> NativeAudioState {
    let entry = QueueEntry(
      src: src,
      id: id,
      metadata: PlaybackMetadata(title: title, artist: artist, artworkURL: artworkURL)
    )
    return try await setQueue(items: [entry], startIndex: 0, startPosition: 0.0)
  }

  /// `sourceId`: the playable folder (playlist, album) this queue plays, for "folder" tracked lists.
  func setQueue(items: [QueueEntry], startIndex: Int, startPosition: Double, sourceId: String? = nil) async throws -> NativeAudioState {
    guard !items.isEmpty else {
      throw NativeAudioRuntimeError.emptyQueue
    }
    guard items.indices.contains(startIndex) else {
      throw NativeAudioRuntimeError.indexOutOfRange
    }

    ensureConfigured()
    try audioSessionController.configurePlaybackCategory()

    let previousQueue = queue
    let previousIndex = queueIndex
    let previousOrder = playOrder
    let previousSourceId = queueSourceId
    queue = items
    queueSourceId = (sourceId?.isEmpty ?? true) ? nil : sourceId
    rebuildPlayOrder(first: startIndex)
    do {
      try await loadQueueEntry(at: startIndex, autoplay: false)
    } catch {
      queue = previousQueue
      queueIndex = previousIndex
      playOrder = previousOrder
      queueSourceId = previousSourceId
      updateTrackCommands()
      throw error
    }
    saveQueueSnapshot()

    if startPosition.isFinite, startPosition > 0 {
      return await seekTo(position: startPosition)
    }
    return snapshot()
  }

  func next() async throws -> NativeAudioState {
    guard hasNextEntry else {
      return snapshot()
    }
    try await moveToNextEntry(autoplay: machine.desiredPlaying)
    return snapshot()
  }

  func previous() async throws -> NativeAudioState {
    // Like other music players: restart the current track unless playback is near its beginning.
    guard snapshot().currentTime <= previousRestartThresholdSeconds, let previousIndex = previousQueueIndex else {
      return await seekTo(position: 0.0)
    }
    try await loadQueueEntry(at: previousIndex, autoplay: machine.desiredPlaying)
    return snapshot()
  }

  func skipTo(index: Int) async throws -> NativeAudioState {
    guard queue.indices.contains(index) else {
      throw NativeAudioRuntimeError.indexOutOfRange
    }
    // Picking a track while shuffled plays it, then shuffles the rest of the queue after it.
    if shuffleEnabled {
      rebuildPlayOrder(first: index)
      saveQueueSnapshot()
    }
    try await loadQueueEntry(at: index, autoplay: machine.desiredPlaying)
    return snapshot()
  }

  func setShuffle(enabled: Bool) -> NativeAudioState {
    // Turning shuffle on (again) always makes a fresh order that starts with the current track.
    if enabled != shuffleEnabled {
      shuffleEnabled = enabled
      rebuildPlayOrder(first: queueIndex >= 0 ? queueIndex : nil)
      updateTrackCommands()
      saveQueueSnapshot()
    }
    emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: true, refreshArtwork: false)
    return snapshot()
  }

  func setRepeatMode(_ mode: RepeatMode) -> NativeAudioState {
    repeatMode = mode
    updateTrackCommands()
    saveQueueSnapshot()
    emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: true, refreshArtwork: false)
    return snapshot()
  }

  /// Adds entries right after the current track (`playNext`) or at the end. With shuffle on they're
  /// placed at the same spot in the play order; the rest of the order stays.
  func setOptions(repeatAddedTracks: Bool?, trackProgress: Bool?) {
    if let repeatAddedTracks {
      self.repeatAddedTracks = repeatAddedTracks
      UserDefaults.standard.set(repeatAddedTracks, forKey: repeatAddedTracksDefaultsKey)
    }
    if let trackProgress {
      itemProgressStore.trackProgress = trackProgress
    }
  }

  /// Pauses after `minutes` (fading out over the last `fadeOutSeconds`), or at the end of the
  /// current track with `endOfTrack`. Replaces a running timer.
  func setSleepTimer(minutes: Double?, endOfTrack: Bool, fadeOutSeconds: Double) -> NativeAudioState {
    clearSleepTimer()
    if endOfTrack {
      sleepTimerEndOfTrack = true
    } else if let minutes {
      let total = max(0, minutes * 60)
      let fade = min(max(0, fadeOutSeconds), total)
      sleepTimerEndsAt = Date().addingTimeInterval(total)
      sleepTimerTask = Task { [weak self] in
        try? await Task.sleep(nanoseconds: UInt64((total - fade) * 1_000_000_000))
        await self?.runSleepFade(seconds: fade)
      }
    }
    emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: true, refreshArtwork: false)
    return snapshot()
  }

  func cancelSleepTimer() -> NativeAudioState {
    clearSleepTimer()
    emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: true, refreshArtwork: false)
    return snapshot()
  }

  private func clearSleepTimer() {
    sleepTimerTask?.cancel()
    sleepTimerTask = nil
    sleepTimerEndsAt = nil
    sleepTimerEndOfTrack = false
    playerAdapter.setVolume(1)
  }

  private func runSleepFade(seconds: Double) async {
    guard !Task.isCancelled, sleepTimerEndsAt != nil else {
      return
    }
    let steps = Int(seconds * 10)
    for step in 0..<steps {
      if Task.isCancelled || sleepTimerEndsAt == nil {
        return
      }
      playerAdapter.setVolume(1 - Float(step + 1) / Float(steps + 1))
      try? await Task.sleep(nanoseconds: 100_000_000)
    }
    guard !Task.isCancelled, sleepTimerEndsAt != nil else {
      return
    }
    _ = await pause()
    clearSleepTimer()
    emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: true, refreshArtwork: false)
  }

  func itemProgress(itemIds: [Int64]?) -> [ItemProgressEntry] {
    itemProgressStore.list(itemIds: itemIds)
  }

  func setItemProgress(_ entries: [ItemProgressEntry], merge: Bool) -> [ItemProgressEntry] {
    itemProgressStore.set(entries, merge: merge)
  }

  private func recordItemProgress(itemId: Int64?, position: Double, duration: Double, completed: Bool) {
    guard let itemId else {
      return
    }
    itemProgressStore.record(itemId: itemId, position: position, duration: duration, completed: completed)
  }

  func addToQueue(items: [QueueEntry], playNext: Bool) async throws -> NativeAudioState {
    guard !items.isEmpty else {
      throw NativeAudioRuntimeError.emptyQueue
    }
    guard !queue.isEmpty else {
      return try await setQueue(items: items, startIndex: 0, startPosition: 0.0)
    }
    let items = items.map { item -> QueueEntry in
      var added = item
      added.addedToQueue = true
      return added
    }

    let at = playNext ? queueIndex + 1 : queue.count
    let playPosition = playNext ? (playOrderPosition ?? -1) + 1 : playOrder.count
    queue.insert(contentsOf: items, at: at)
    if at <= queueIndex {
      queueIndex += items.count
    }
    playOrder = shuffleEnabled
      ? QueueOrder.insert(playOrder, at: at, count: items.count, playPosition: playPosition)
      : Array(queue.indices)
    queueChanged()
    return snapshot()
  }

  /// Removing the current track moves on to the next one in play order.
  func removeFromQueue(index: Int) async throws -> NativeAudioState {
    guard queue.indices.contains(index) else {
      throw NativeAudioRuntimeError.indexOutOfRange
    }
    let removingCurrent = index == queueIndex
    // What plays instead when the current track is removed (as a pre-removal index).
    var replacement: Int?
    if removingCurrent, let position = playOrderPosition {
      if position + 1 < playOrder.count {
        replacement = playOrder[position + 1]
      } else if repeatMode == .all, let first = playOrder.first, first != index {
        replacement = first
      }
    }

    queue.remove(at: index)
    playOrder = shuffleEnabled ? QueueOrder.remove(playOrder, at: index) : Array(queue.indices)
    if index < queueIndex {
      queueIndex -= 1
    }

    guard removingCurrent else {
      queueChanged()
      return snapshot()
    }
    if queue.isEmpty {
      queueIndex = -1
      playerAdapter.pause()
      machine.markEnded()
      queueChanged()
      return snapshot()
    }
    if let replacement {
      queueChanged()
      try await loadQueueEntry(at: replacement > index ? replacement - 1 : replacement, autoplay: machine.desiredPlaying)
      return snapshot()
    }
    // The removed track was the last one: stop on the new last track, like the queue ended.
    queueChanged()
    try await loadQueueEntry(at: playOrder.last ?? queue.count - 1, autoplay: false)
    machine.markEnded()
    emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: true, refreshArtwork: false)
    return snapshot()
  }

  /// Moves a queue entry (list indices). With shuffle on the play order stays the same; without
  /// shuffle the list order is the play order.
  func moveInQueue(from: Int, to: Int) async throws -> NativeAudioState {
    guard queue.indices.contains(from), queue.indices.contains(to) else {
      throw NativeAudioRuntimeError.indexOutOfRange
    }
    guard from != to else {
      return snapshot()
    }
    let entry = queue.remove(at: from)
    queue.insert(entry, at: to)
    if queueIndex >= 0 {
      queueIndex = QueueOrder.movedIndex(queueIndex, from: from, to: to)
    }
    playOrder = shuffleEnabled ? QueueOrder.move(playOrder, from: from, to: to) : Array(queue.indices)
    queueChanged()
    return snapshot()
  }

  func getQueue() -> (items: [QueueEntry], currentIndex: Int, playOrder: [Int]) {
    (queue, queueIndex, playOrder)
  }

  /// Loads the last saved queue (items, track, position, shuffle order, repeat mode), paused.
  /// Returns nil when nothing was saved.
  func restoreLastQueue() async throws -> NativeAudioState? {
    guard let saved = queueSnapshotStore.load() else {
      return nil
    }
    ensureConfigured()
    try audioSessionController.configurePlaybackCategory()

    queue = saved.items
    queueSourceId = saved.sourceId
    shuffleEnabled = saved.shuffle
    repeatMode = saved.repeatMode
    if shuffleEnabled, let order = saved.playOrder {
      playOrder = order
    } else {
      rebuildPlayOrder(first: saved.index)
    }
    try await loadQueueEntry(at: saved.index, autoplay: false)
    saveQueueSnapshot()
    if saved.position > 0 {
      return await seekTo(position: saved.position)
    }
    return snapshot()
  }

  func play() async throws -> NativeAudioState {
    ensureConfigured()
    try audioSessionController.configurePlaybackCategory()
    try audioSessionController.setActive(true)
    machine.setDesiredPlaying(true)

    if machine.didReachEnd {
      machine.clearEnded()
      let pending = machine.beginSeek(position: 0.0, shouldResume: true, originPosition: playerAdapter.currentTimeSeconds())
      playerAdapter.seek(to: 0.0, sourceRevision: pending.sourceRevision, seekRevision: pending.revision)
    }

    machine.clearError()
    playerAdapter.play(rate: machine.playbackRate)
    logStartIfNeeded()

    emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: true, refreshArtwork: false)
    return snapshot()
  }

  func pendingPlaybackEvents() -> [PlaybackEventPayload] {
    playbackEventLog.pending()
  }

  func acknowledgePlaybackEvents(ids: Set<String>) {
    playbackEventLog.acknowledge(ids: ids)
  }

  func pause() async -> NativeAudioState {
    machine.setDesiredPlaying(false)
    machine.clearPendingSeek()
    playerAdapter.pause()
    saveQueuePosition(force: true)
    recordItemProgress(
      itemId: machine.currentStoryId,
      position: playerAdapter.currentTimeSeconds(),
      duration: playerAdapter.durationSeconds(),
      completed: false
    )

    emitState(trigger: .transition, forcePersistCheckpoint: true, forceEmit: true, refreshArtwork: false)
    return snapshot()
  }

  func seekTo(position: Double) async -> NativeAudioState {
    let safePosition = max(0.0, position)
    let shouldResume = machine.desiredPlaying

    machine.clearEnded()
    let pending = machine.beginSeek(position: safePosition, shouldResume: shouldResume, originPosition: playerAdapter.currentTimeSeconds())

    if !shouldResume {
      playerAdapter.pause()
    }

    playerAdapter.seek(to: safePosition, sourceRevision: pending.sourceRevision, seekRevision: pending.revision)

    emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: true, refreshArtwork: false)
    return snapshot()
  }

  func setRate(rate: Double) async throws -> NativeAudioState {
    guard rate.isFinite, rate > 0 else {
      throw NativeAudioRuntimeError.invalidRate
    }

    machine.setPlaybackRate(rate)
    playerAdapter.setRate(rate)

    emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: true, refreshArtwork: false)
    return snapshot()
  }

  func setSkipInterval(seconds: Double) throws -> NativeAudioState {
    guard seconds.isFinite, seconds >= 0 else {
      throw NativeAudioRuntimeError.invalidSkipInterval
    }
    remoteCommandController.setSkipInterval(seconds: seconds)
    return snapshot()
  }

  func getState() -> NativeAudioState {
    snapshot()
  }

  func getProgressCheckpoint() -> NativeAudioProgressCheckpoint? {
    checkpointStore.read()
  }

  func clearProgressCheckpoint() {
    checkpointStore.clear()
  }

  func dispose() async {
    let preDisposeSnapshot = snapshot()
    checkpointStore.persistIfNeeded(snapshot: preDisposeSnapshot, storyId: machine.currentStoryId, force: true)
    saveQueuePosition(force: true)

    remoteCommandController.unregister()
    audioSessionController.unregisterObservers()
    unregisterAppLifecycleObservers()

    nowPlayingController.clear()
    playerAdapter.dispose()

    await sourceResolver.cleanupAll()

    machine.resetAll()
    queue = []
    queueIndex = -1
    playOrder = []
    queueLoadRevision += 1
    wasPlayingBeforeInterruption = false
    lastEmittedState = nil
    lastProgressTickEmitAt = .distantPast
    isConfigured = false

    try? audioSessionController.setActive(false)
    emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: true, refreshArtwork: false)
  }

  private var playOrderPosition: Int? {
    playOrder.firstIndex(of: queueIndex)
  }

  /// Whether there is a track after the current one, including the wrap-around with repeat all.
  /// Like ExoPlayer, repeat one doesn't block skipping to the next track.
  private var hasNextEntry: Bool {
    guard let position = playOrderPosition else {
      return false
    }
    return position + 1 < playOrder.count || repeatMode == .all
  }

  private var previousQueueIndex: Int? {
    guard let position = playOrderPosition else {
      return nil
    }
    if position > 0 {
      return playOrder[position - 1]
    }
    return repeatMode == .all ? playOrder.last : nil
  }

  private func rebuildPlayOrder(first: Int?, recent: Set<Int> = []) {
    guard shuffleEnabled else {
      playOrder = Array(queue.indices)
      return
    }
    let entries = queue
    playOrder = ShuffleOrderBuilder.build(
      count: entries.count,
      first: first,
      recent: recent,
      artistOf: { entries[$0].metadata.artist }
    )
  }

  /// Loads the track after the current one in play order. At the end of the order with repeat all,
  /// it wraps to the start, and with shuffle on it first builds a new order for the next pass, so
  /// passes don't repeat the same order.
  private func moveToNextEntry(autoplay: Bool) async throws {
    guard let position = playOrderPosition else {
      return
    }
    if position + 1 < playOrder.count {
      try await loadQueueEntry(at: playOrder[position + 1], autoplay: autoplay)
      return
    }
    guard repeatMode == .all, !playOrder.isEmpty else {
      return
    }
    if !repeatAddedTracks, queue.contains(where: { $0.addedToQueue }) {
      dropAddedTracks()
      guard !queue.isEmpty else {
        return
      }
    }
    if shuffleEnabled {
      rebuildPlayOrder(first: nil, recent: ShuffleOrderBuilder.recentTail(playOrder))
    }
    saveQueueSnapshot()
    try await loadQueueEntry(at: playOrder[0], autoplay: autoplay)
  }

  /// Swaps the player item to `queue[index]`. If a newer load starts while the source is being
  /// resolved, the newer one wins and this one returns without touching the player.
  private func loadQueueEntry(at index: Int, autoplay: Bool) async throws {
    let entry = queue[index]
    queueLoadRevision += 1
    let loadRevision = queueLoadRevision

    let playbackURL = try await sourceResolver.resolvePlayableURL(src: entry.src)
    guard loadRevision == queueLoadRevision else {
      return
    }

    // Leaving a track that started but didn't finish.
    if currentStartLogged {
      logPlaybackEvent(type: "skip", itemId: machine.currentStoryId, position: playerAdapter.currentTimeSeconds())
      recordItemProgress(
        itemId: machine.currentStoryId,
        position: playerAdapter.currentTimeSeconds(),
        duration: playerAdapter.durationSeconds(),
        completed: false
      )
    }
    currentStartLogged = false
    countedLists.removeAll()

    queueIndex = index
    let sourceRevision = machine.advanceSourceRevision()
    machine.setStoryId(entry.id)
    machine.setMetadata(entry.metadata)

    wasPlayingBeforeInterruption = false
    playerAdapter.pause()
    playerAdapter.replaceCurrentItem(url: playbackURL, sourceRevision: sourceRevision)
    updateTrackCommands()
    saveQueuePosition(force: true)

    emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: true, refreshArtwork: true)

    if autoplay {
      _ = try await play()
    }
  }

  private func updateTrackCommands() {
    remoteCommandController.setTrackCommandsEnabled(hasNext: hasNextEntry, hasPrevious: queueIndex >= 0)
  }

  /// Removes the tracks added with addToQueue before the queue repeats (repeatAddedTracks off).
  /// The current track may be one of them: it's about to be replaced by the first of the new pass.
  private func dropAddedTracks() {
    for index in queue.indices.reversed() where queue[index].addedToQueue {
      queue.remove(at: index)
      playOrder = QueueOrder.remove(playOrder, at: index)
      if index < queueIndex {
        queueIndex -= 1
      } else if index == queueIndex {
        queueIndex = -1
      }
    }
    if !shuffleEnabled {
      playOrder = Array(queue.indices)
    }
  }

  private func logStartIfNeeded() {
    guard !currentStartLogged, queueIndex >= 0 else {
      return
    }
    currentStartLogged = true
    logPlaybackEvent(type: "start", itemId: machine.currentStoryId, position: playerAdapter.currentTimeSeconds())
    countPlay(itemId: machine.currentStoryId, position: playerAdapter.currentTimeSeconds(), completed: false)
  }

  /// Counts the current play towards the tracked lists whose countAfterSeconds it reached
  /// (`completed`: it played to the end, which always counts). Each list once per play.
  private func countPlay(itemId: Int64?, position: Double, completed: Bool) {
    for list in trackedListsStore.configs() where !countedLists.contains(list.id) {
      if !completed, position < (list.countAfterSeconds ?? 0) {
        continue
      }
      let value: TrackedListValue
      if list.track == "item" {
        guard let itemId else { continue }
        value = .item(itemId)
      } else {
        guard let queueSourceId else { continue }
        value = .folder(queueSourceId)
      }
      countedLists.insert(list.id)
      guard let change = trackedListsStore.record(config: list, value: value) else {
        continue
      }
      if let emitter {
        if Thread.isMainThread {
          emitter.emitTrackedListChange(change)
        } else {
          DispatchQueue.main.sync {
            emitter.emitTrackedListChange(change)
          }
        }
      }
    }
  }

  func setTrackedLists(_ configs: [TrackedListConfig]) throws {
    try trackedListsStore.setConfigs(configs)
    countedLists.removeAll()
  }

  func trackedList(id: String) -> [TrackedListEntry] {
    trackedListsStore.entries(listId: id)
  }

  func setTrackedList(id: String, entries: [TrackedListEntry], merge: Bool) throws -> [TrackedListEntry] {
    try trackedListsStore.set(listId: id, entries: entries, merge: merge)
  }

  func pendingTrackedListChanges() -> [TrackedListChange] {
    trackedListsStore.pendingChanges()
  }

  func acknowledgeTrackedListChanges(ids: Set<String>) {
    trackedListsStore.acknowledge(ids: ids)
  }

  private func logPlaybackEvent(type: String, itemId: Int64?, position: Double) {
    guard let itemId else {
      return
    }
    let event = playbackEventLog.record(type: type, itemId: itemId, position: position)
    if let emitter {
      if Thread.isMainThread {
        emitter.emitPlaybackEvent(event)
      } else {
        DispatchQueue.main.sync {
          emitter.emitPlaybackEvent(event)
        }
      }
    }
  }

  /// After adding, removing or moving entries without changing the current track.
  private func queueChanged() {
    updateTrackCommands()
    saveQueueSnapshot()
    emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: true, refreshArtwork: false)
  }

  private func saveQueueSnapshot() {
    queueSnapshotStore.saveQueue(queue, shuffle: shuffleEnabled, playOrder: playOrder, repeatMode: repeatMode, sourceId: queueSourceId)
    saveQueuePosition(force: true)
  }

  private func saveQueuePosition(force: Bool) {
    queueSnapshotStore.savePosition(index: queueIndex, position: playerAdapter.currentTimeSeconds(), force: force)
  }

  private func ensureConfigured() {
    if isConfigured {
      return
    }

    playerAdapter.ensurePlayer()
    playerAdapter.setEventHandler { [weak self] event in
      guard let self else { return }
      Task {
        await self.handlePlayerEvent(event)
      }
    }

    audioSessionController.registerObserversIfNeeded { [weak self] event in
      guard let self else { return }
      Task {
        await self.handleAudioSessionEvent(event)
      }
    }

    remoteCommandController.registerIfNeeded { [weak self] event in
      guard let self else { return }
      Task {
        await self.handleRemoteCommandEvent(event)
      }
    }

    registerAppLifecycleObserversIfNeeded()
    isAppInForeground = true

    isConfigured = true
  }

  private func registerAppLifecycleObserversIfNeeded() {
    guard appDidBecomeActiveObserver == nil, appDidEnterBackgroundObserver == nil else {
      return
    }

    let center = NotificationCenter.default
    appDidBecomeActiveObserver = center.addObserver(
      forName: UIApplication.didBecomeActiveNotification,
      object: nil,
      queue: .main
    ) { [weak self] _ in
      guard let self else { return }
      Task {
        await self.handleAppLifecycleChanged(isForeground: true)
      }
    }

    appDidEnterBackgroundObserver = center.addObserver(
      forName: UIApplication.didEnterBackgroundNotification,
      object: nil,
      queue: .main
    ) { [weak self] _ in
      guard let self else { return }
      Task {
        await self.handleAppLifecycleChanged(isForeground: false)
      }
    }
  }

  private func unregisterAppLifecycleObservers() {
    let center = NotificationCenter.default

    if let appDidBecomeActiveObserver {
      center.removeObserver(appDidBecomeActiveObserver)
      self.appDidBecomeActiveObserver = nil
    }

    if let appDidEnterBackgroundObserver {
      center.removeObserver(appDidEnterBackgroundObserver)
      self.appDidEnterBackgroundObserver = nil
    }
  }

  private func handleAppLifecycleChanged(isForeground: Bool) {
    isAppInForeground = isForeground
    emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: false, refreshArtwork: false)
  }

  private func handlePlayerEvent(_ event: PlayerEvent) async {
    switch event {
    case let .timeControlChanged(sourceRevision):
      guard sourceRevision == machine.sourceRevision else { return }
      if playerAdapter.isActuallyPlaying(), machine.didReachEnd {
        machine.clearEnded()
      }
      emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: false, refreshArtwork: false)

    case let .itemStatusChanged(sourceRevision, status, error):
      guard sourceRevision == machine.sourceRevision else { return }
      switch status {
      case .readyToPlay:
        machine.clearError()
      case .failed:
        machine.markError(error ?? "failed to load audio source")
      case .unknown:
        break
      @unknown default:
        break
      }
      emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: false, refreshArtwork: false)

    case let .durationChanged(sourceRevision):
      guard sourceRevision == machine.sourceRevision else { return }
      emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: false, refreshArtwork: false)

    case let .progress(sourceRevision, _):
      guard sourceRevision == machine.sourceRevision else { return }
      // Ignore progress updates until the active seek revision commits at target.
      if let pendingSeek = machine.pendingSeek {
        if pendingSeek.sourceRevision == sourceRevision, hasSeekSettled(pendingSeek: pendingSeek, currentTime: playerAdapter.currentTimeSeconds()) {
          _ = machine.resolveSeek(sourceRevision: pendingSeek.sourceRevision, seekRevision: pendingSeek.revision)
          machine.clearError()
          emitState(trigger: .transition, forcePersistCheckpoint: true, forceEmit: true, refreshArtwork: false)
        }
        return
      }
      emitState(trigger: .progressTick, forcePersistCheckpoint: false, forceEmit: false, refreshArtwork: false)

    case let .didReachEnd(sourceRevision):
      guard sourceRevision == machine.sourceRevision else { return }
      if machine.pendingSeek != nil {
        return
      }
      logPlaybackEvent(type: "complete", itemId: machine.currentStoryId, position: playerAdapter.durationSeconds())
      countPlay(itemId: machine.currentStoryId, position: playerAdapter.durationSeconds(), completed: true)
      recordItemProgress(
        itemId: machine.currentStoryId,
        position: playerAdapter.durationSeconds(),
        duration: playerAdapter.durationSeconds(),
        completed: true
      )
      currentStartLogged = false
      countedLists.removeAll()
      if sleepTimerEndOfTrack {
        // End-of-track sleep timer: stop here, with the next track ready.
        clearSleepTimer()
        machine.setDesiredPlaying(false)
        if hasNextEntry {
          do {
            try await moveToNextEntry(autoplay: false)
          } catch {
            machine.markError(error.localizedDescription)
          }
        } else {
          machine.markEnded()
        }
        emitState(trigger: .transition, forcePersistCheckpoint: true, forceEmit: true, refreshArtwork: false)
        return
      }
      if repeatMode == .one {
        // Same path as pressing play after the end: seek to 0 and play.
        machine.markEnded()
        do {
          _ = try await play()
        } catch {
          machine.markError(error.localizedDescription)
          emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: true, refreshArtwork: false)
        }
        return
      }
      if hasNextEntry {
        checkpointStore.persistIfNeeded(snapshot: snapshot(), storyId: machine.currentStoryId, force: true)
        do {
          try await moveToNextEntry(autoplay: true)
        } catch {
          machine.markError(error.localizedDescription)
          emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: true, refreshArtwork: false)
        }
        return
      }
      machine.markEnded()
      emitState(trigger: .transition, forcePersistCheckpoint: true, forceEmit: true, refreshArtwork: false)

    case let .failedToEnd(sourceRevision, error):
      guard sourceRevision == machine.sourceRevision else { return }
      if machine.pendingSeek != nil {
        return
      }
      machine.markError(error ?? "failed to play audio")
      emitState(trigger: .transition, forcePersistCheckpoint: true, forceEmit: true, refreshArtwork: false)

    case let .seekCompleted(sourceRevision, seekRevision, finished, _):
      guard finished else {
        if machine.cancelSeek(sourceRevision: sourceRevision, seekRevision: seekRevision) {
          emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: true, refreshArtwork: false)
        }
        return
      }
      guard let pendingSeek = machine.pendingSeek else {
        return
      }
      guard pendingSeek.sourceRevision == sourceRevision, pendingSeek.revision == seekRevision else {
        return
      }

      if pendingSeek.shouldResume && machine.desiredPlaying {
        playerAdapter.play(rate: machine.playbackRate)
      } else {
        playerAdapter.pause()
      }

      if !pendingSeek.shouldResume || hasSeekSettled(pendingSeek: pendingSeek, currentTime: playerAdapter.currentTimeSeconds()) {
        _ = machine.resolveSeek(sourceRevision: sourceRevision, seekRevision: seekRevision)
      }

      machine.clearError()
      emitState(trigger: .transition, forcePersistCheckpoint: true, forceEmit: true, refreshArtwork: false)
    }
  }

  private func handleAudioSessionEvent(_ event: AudioSessionEvent) async {
    switch event {
    case .interruptionBegan:
      wasPlayingBeforeInterruption = machine.desiredPlaying
      machine.clearPendingSeek()
      if wasPlayingBeforeInterruption {
        machine.setDesiredPlaying(false)
        playerAdapter.pause()
      }
      emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: true, refreshArtwork: false)

    case let .interruptionEnded(shouldResume):
      defer { wasPlayingBeforeInterruption = false }
      guard wasPlayingBeforeInterruption, shouldResume else {
        emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: true, refreshArtwork: false)
        return
      }

      do {
        _ = try await play()
      } catch {
        machine.markError(error.localizedDescription)
        emitState(trigger: .transition, forcePersistCheckpoint: false, forceEmit: true, refreshArtwork: false)
      }

    case .oldDeviceUnavailable:
      machine.clearPendingSeek()
      machine.setDesiredPlaying(false)
      if playerAdapter.isActuallyPlaying() {
        playerAdapter.pause()
      }
      emitState(trigger: .transition, forcePersistCheckpoint: true, forceEmit: true, refreshArtwork: false)
    }
  }

  private func handleRemoteCommandEvent(_ event: RemoteCommandEvent) async {
    switch event {
    case .play:
      _ = try? await play()
    case .pause:
      _ = await pause()
    case .toggle:
      if machine.desiredPlaying {
        _ = await pause()
      } else {
        _ = try? await play()
      }
    case let .seek(position):
      _ = await seekTo(position: position)
    case let .seekDelta(delta):
      var target = snapshot().currentTime + delta
      let duration = playerAdapter.durationSeconds()
      if duration.isFinite, duration > 0 {
        target = min(target, duration)
      }
      _ = await seekTo(position: target)
    case .nextTrack:
      _ = try? await next()
    case .previousTrack:
      _ = try? await previous()
    }
  }

  private func snapshot() -> NativeAudioState {
    machine.makeSnapshot(
      rawCurrentTime: playerAdapter.currentTimeSeconds(),
      rawDuration: playerAdapter.durationSeconds(),
      isActuallyPlaying: playerAdapter.isActuallyPlaying(),
      isBuffering: playerAdapter.isBuffering(),
      queueIndex: queueIndex,
      queueLength: queue.count,
      shuffle: shuffleEnabled,
      repeatMode: repeatMode,
      sleepTimerEndsAtMs: sleepTimerEndsAt.map { Int64($0.timeIntervalSince1970 * 1000) },
      sleepTimerEndOfTrack: sleepTimerEndOfTrack
    )
  }

  private func emitState(
    trigger: EmitTrigger,
    forcePersistCheckpoint: Bool,
    forceEmit: Bool,
    refreshArtwork: Bool
  ) {
    let state = snapshot()
    let now = Date()

    checkpointStore.persistIfNeeded(snapshot: state, storyId: machine.currentStoryId, force: forcePersistCheckpoint)
    queueSnapshotStore.savePosition(index: queueIndex, position: state.currentTime, force: false, now: now)
    if currentStartLogged {
      countPlay(itemId: machine.currentStoryId, position: state.currentTime, completed: false)
      if state.isPlaying, now.timeIntervalSince(lastItemProgressSavedAt) >= 15 {
        lastItemProgressSavedAt = now
        recordItemProgress(itemId: machine.currentStoryId, position: state.currentTime, duration: state.duration, completed: false)
      }
    }

    let shouldEmit = forceEmit || shouldEmitState(state, trigger: trigger, now: now)
    if shouldEmit {
      if let emitter {
        if Thread.isMainThread {
          emitter.emitNativeAudioState(state)
        } else {
          DispatchQueue.main.sync {
            emitter.emitNativeAudioState(state)
          }
        }
      }
      lastEmittedState = state
      if trigger == .progressTick {
        lastProgressTickEmitAt = now
      }
    }

    let shouldUpdateNowPlaying = trigger != .progressTick || shouldEmit || forceEmit
    if shouldUpdateNowPlaying {
      nowPlayingController.update(state: state, metadata: machine.metadata, refreshArtwork: refreshArtwork)
    }
  }

  private func shouldEmitState(_ next: NativeAudioState, trigger: EmitTrigger, now: Date) -> Bool {
    guard let lastEmittedState else {
      return true
    }

    if trigger == .progressTick {
      if !next.isPlaying {
        return !isSameState(lhs: lastEmittedState, rhs: next)
      }
      let interval = isAppInForeground ? foregroundProgressEmitIntervalSeconds : backgroundProgressEmitIntervalSeconds
      return now.timeIntervalSince(lastProgressTickEmitAt) >= interval
    }

    return !isSameState(lhs: lastEmittedState, rhs: next)
  }

  private func isSameState(lhs: NativeAudioState, rhs: NativeAudioState) -> Bool {
    lhs.status == rhs.status
      && lhs.currentTime == rhs.currentTime
      && lhs.duration == rhs.duration
      && lhs.isPlaying == rhs.isPlaying
      && lhs.buffering == rhs.buffering
      && lhs.rate == rhs.rate
      && lhs.queueIndex == rhs.queueIndex
      && lhs.queueLength == rhs.queueLength
      && lhs.currentId == rhs.currentId
      && lhs.shuffle == rhs.shuffle
      && lhs.repeatMode == rhs.repeatMode
      && lhs.error == rhs.error
  }

  private func hasSeekSettled(pendingSeek: PendingSeekContext, currentTime: Double) -> Bool {
    let direction = pendingSeek.targetPosition - pendingSeek.originPosition
    if direction > 0 {
      return currentTime + seekCommitEpsilonSeconds >= pendingSeek.targetPosition
    }
    if direction < 0 {
      return currentTime - seekCommitEpsilonSeconds <= pendingSeek.targetPosition
    }
    return abs(currentTime - pendingSeek.targetPosition) <= seekCommitEpsilonSeconds
  }

  private func onMain<T>(_ block: () -> T) -> T {
    if Thread.isMainThread {
      return block()
    }
    return DispatchQueue.main.sync(execute: block)
  }
}
