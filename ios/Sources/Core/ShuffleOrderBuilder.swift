import Foundation

/// Builds shuffled play orders (permutations of queue indices) that feel random to a listener:
/// - every track plays once per pass,
/// - tracks from the end of the previous pass are kept out of the start of the next one,
/// - where possible, two tracks by the same artist don't play back to back.
///
/// Mirrors `ShuffleOrderBuilder.kt` on Android.
enum ShuffleOrderBuilder {
  /// - Parameters:
  ///   - count: queue length.
  ///   - first: index to play first (the current track), or nil.
  ///   - recent: indices played at the end of the previous pass; they are kept out of the
  ///     first `recent.count` slots after `first`.
  ///   - artistOf: artist of each index, used to spread artists apart.
  static func build(
    count: Int,
    first: Int?,
    recent: Set<Int> = [],
    artistOf: (Int) -> String? = { _ in nil }
  ) -> [Int] {
    guard count > 0 else {
      return []
    }
    let pinned = first.flatMap { (0..<count).contains($0) ? $0 : nil }
    let rest = (0..<count).filter { $0 != pinned }
    let fresh = rest.filter { !recent.contains($0) }.shuffled()
    let stale = rest.filter { recent.contains($0) }

    let headCount = min(fresh.count, stale.count)
    var head = Array(fresh.prefix(headCount))
    var tail = (Array(fresh.dropFirst(headCount)) + stale).shuffled()

    var artists: [Int: String?] = [:]
    let normalizedArtist = { (index: Int) -> String? in
      if let cached = artists[index] {
        return cached
      }
      let artist = artistOf(index)?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
      let normalized = (artist?.isEmpty ?? true) ? nil : artist
      artists[index] = normalized
      return normalized
    }
    let pinnedArtist = pinned.flatMap(normalizedArtist)
    spreadArtists(&head, previousArtist: pinnedArtist, artistOf: normalizedArtist)
    spreadArtists(&tail, previousArtist: head.last.flatMap(normalizedArtist) ?? pinnedArtist, artistOf: normalizedArtist)

    return (pinned.map { [$0] } ?? []) + head + tail
  }

  /// The last ~20% of `order`: what the next pass should not start with.
  static func recentTail(_ order: [Int]) -> Set<Int> {
    guard order.count > 2 else {
      return []
    }
    return Set(order.suffix(max(1, order.count / 5)))
  }

  /// Reorders the already shuffled `items` so no two neighbours share an artist, when that's possible.
  /// Walks the shuffled list and takes the first track whose artist differs from the previous one,
  /// except when one artist fills more than half of what's left: that artist has to go now, or its
  /// tracks would end up back to back at the end.
  private static func spreadArtists(_ items: inout [Int], previousArtist: String?, artistOf: (Int) -> String?) {
    var remaining = items
    var counts: [String: Int] = [:]
    for index in remaining {
      if let artist = artistOf(index) {
        counts[artist, default: 0] += 1
      }
    }

    items.removeAll(keepingCapacity: true)
    var previous = previousArtist
    while !remaining.isEmpty {
      let left = remaining.count
      let forced = counts.first { $0.key != previous && $0.value * 2 > left }?.key
      let pick: Int
      if let forced {
        pick = remaining.firstIndex { artistOf($0) == forced } ?? 0
      } else {
        pick = remaining.firstIndex { previous == nil || artistOf($0) != previous } ?? 0
      }
      let index = remaining.remove(at: pick)
      let artist = artistOf(index)
      if let artist {
        counts[artist, default: 1] -= 1
      }
      items.append(index)
      previous = artist
    }
  }
}
