import Foundation

/// Keeps a play order (a permutation of queue indices, e.g. the shuffle order) in step with edits
/// to the queue list. Mirrors `QueueOrder.kt` on Android.
enum QueueOrder {
  /// `count` entries were inserted into the list at `at`. Returns the new order, with the new
  /// entries placed at `playPosition` in the play order (clamped to its bounds).
  static func insert(_ order: [Int], at: Int, count: Int, playPosition: Int) -> [Int] {
    var shifted = order.map { $0 >= at ? $0 + count : $0 }
    let position = min(max(playPosition, 0), shifted.count)
    shifted.insert(contentsOf: Array(at..<(at + count)), at: position)
    return shifted
  }

  /// The list entry at `at` was removed.
  static func remove(_ order: [Int], at: Int) -> [Int] {
    order.filter { $0 != at }.map { $0 > at ? $0 - 1 : $0 }
  }

  /// The list entry at `from` was moved to `to`; its place in the play order doesn't change.
  static func move(_ order: [Int], from: Int, to: Int) -> [Int] {
    order.map { movedIndex($0, from: from, to: to) }
  }

  /// Where list index `index` ends up after moving `from` to `to`.
  static func movedIndex(_ index: Int, from: Int, to: Int) -> Int {
    if index == from {
      return to
    }
    if from < to, index > from, index <= to {
      return index - 1
    }
    if from > to, index >= to, index < from {
      return index + 1
    }
    return index
  }
}
