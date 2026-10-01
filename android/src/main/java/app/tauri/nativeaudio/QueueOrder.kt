package app.tauri.nativeaudio

/**
 * Keeps a play order (a permutation of queue indices, e.g. the shuffle order) in step with edits
 * to the queue list. ExoPlayer would put inserted or moved items at random places in its shuffle
 * order; these keep the existing order and put new items exactly where they were asked for.
 */
internal object QueueOrder {
    /**
     * [count] entries were inserted into the list at [at]. Returns the new order, with the new
     * entries placed at [playPosition] in the play order (clamped to its bounds).
     */
    fun insert(order: IntArray, at: Int, count: Int, playPosition: Int): IntArray {
        val shifted = order.map { if (it >= at) it + count else it }.toMutableList()
        shifted.addAll(playPosition.coerceIn(0, shifted.size), (at until at + count).toList())
        return shifted.toIntArray()
    }

    /** The list entry at [at] was removed. */
    fun remove(order: IntArray, at: Int): IntArray =
        order.filter { it != at }.map { if (it > at) it - 1 else it }.toIntArray()

    /** The list entry at [from] was moved to [to]; its place in the play order doesn't change. */
    fun move(order: IntArray, from: Int, to: Int): IntArray =
        order.map { movedIndex(it, from, to) }.toIntArray()

    /** Where list index [index] ends up after moving [from] to [to]. */
    fun movedIndex(index: Int, from: Int, to: Int): Int = when {
        index == from -> to
        from < to && index in (from + 1)..to -> index - 1
        from > to && index in to until from -> index + 1
        else -> index
    }
}
