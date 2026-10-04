package app.tauri.nativeaudio

import kotlin.random.Random

/**
 * updateQueue: the queue's playlist changed, so work out the new queue from the old one. The same
 * rules as the desktop player (Queue::update):
 *
 * - entries are matched by item id (the same id twice: in order); what stays keeps its place in the
 *   play order, new items are added and missing ones removed;
 * - entries added with addToQueue stay, after the entry they followed;
 * - the current entry stays even if it left the playlist (it's dropped once playback moves on).
 *
 * [keepRemoved]: entries no longer in the playlist stay where they are. [newItems]: "inPlace" (at
 * their place in the playlist; with shuffle somewhere in the rest of this pass), "end" (with
 * shuffle: the end of the play order, shuffled among themselves) or "next" (right after the current
 * entry, in playlist order). [keepQueueOrder]: keep the queue's list order instead of the playlist's
 * (new items in place go after the item before them in the playlist).
 */
internal object QueueUpdate {
    sealed class Slot {
        /** An old entry that's in the new playlist too, with [item]'s (possibly updated) details. */
        data class Matched(val old: Int, val item: Int) : Slot()

        /** A new item. */
        data class New(val item: Int) : Slot()

        /** An old entry kept although it's not in the playlist (added with addToQueue, or playing). */
        data class Kept(val old: Int) : Slot()
    }

    class Plan(
        /** The new queue, in list order. */
        val list: List<Slot>,
        /** The new play order (indices into [list]); only meaningful with shuffle. */
        val order: IntArray,
        /** The current entry isn't in the playlist anymore: drop it once playback moves on. */
        val currentLeft: Boolean,
        /** Items new to the playlist. */
        val added: Int,
        /** Items no longer in the playlist (also when kept, or playing to the end). */
        val removed: Int,
    )

    private fun oldIndexOf(slot: Slot) = when (slot) {
        is Slot.Matched -> slot.old
        is Slot.Kept -> slot.old
        is Slot.New -> -1
    }

    fun plan(
        oldIds: List<Long?>,
        oldAdded: List<Boolean>,
        oldOrder: IntArray,
        current: Int?,
        newIds: List<Long?>,
        shuffle: Boolean,
        keepRemoved: Boolean = false,
        newItems: String = "inPlace",
        keepQueueOrder: Boolean = false,
        random: Random = Random.Default,
    ): Plan {
        val used = BooleanArray(oldIds.size)
        // The new playlist: old entries where matched, new ones otherwise.
        val playlist = newIds.mapIndexed { item, id ->
            val match = if (id == null) -1 else oldIds.indices.firstOrNull { !used[it] && !oldAdded[it] && oldIds[it] == id } ?: -1
            if (match >= 0) {
                used[match] = true
                Slot.Matched(match, item)
            } else {
                Slot.New(item)
            }
        }
        val added = playlist.count { it is Slot.New }
        val removed = oldIds.indices.count { !used[it] && !oldAdded[it] }
        // Old entries that stay although they're not in the playlist.
        fun kept(old: Int) = !used[old] && (oldAdded[old] || keepRemoved || old == current)
        val currentLeft = current != null && kept(current) && !oldAdded[current] && !keepRemoved

        val list = ArrayList<Slot>()
        val later = ArrayList<Slot>()
        if (keepQueueOrder) {
            // The queue's order: old entries that stay, where they were.
            val matchedByOld = playlist.filterIsInstance<Slot.Matched>().associateBy { it.old }
            for (old in oldIds.indices) {
                if (used[old]) list.add(matchedByOld.getValue(old)) else if (kept(old)) list.add(Slot.Kept(old))
            }
            for ((p, slot) in playlist.withIndex()) {
                if (slot !is Slot.New) continue
                if (newItems != "inPlace") {
                    later.add(slot)
                    continue
                }
                // After the item before it in the playlist (that's in the queue).
                val at = (p - 1 downTo 0).firstNotNullOfOrNull { q -> list.indexOf(playlist[q]).takeIf { it >= 0 } }?.plus(1) ?: 0
                list.add(at, slot)
            }
        } else {
            // The playlist's order; kept entries after the nearest entry before them (in the old list).
            for (slot in playlist) if (slot is Slot.New && newItems != "inPlace") later.add(slot) else list.add(slot)
            for (old in oldIds.indices) {
                if (!kept(old)) continue
                val at = (old - 1 downTo 0).firstNotNullOfOrNull { before ->
                    list.indexOfFirst { oldIndexOf(it) == before }.takeIf { it >= 0 }
                }?.plus(1) ?: 0
                list.add(at, Slot.Kept(old))
            }
        }
        when (newItems) {
            "end" -> list.addAll(later)
            "next" -> {
                val at = current?.let { c -> list.indexOfFirst { oldIndexOf(it) == c } }?.takeIf { it >= 0 }?.plus(1) ?: list.size
                list.addAll(at, later)
            }
        }

        val order = if (shuffle) {
            val positionOfOld = HashMap<Int, Int>()
            list.forEachIndexed { i, slot -> oldIndexOf(slot).takeIf { it >= 0 }?.let { positionOfOld[it] = i } }
            val order = oldOrder.toList().mapNotNull { positionOfOld[it] }.toMutableList()
            val start = current?.let { positionOfOld[it] }?.let { order.indexOf(it) + 1 } ?: 0
            // New entries in list order, so "next" keeps the playlist's order.
            val fresh = list.indices.filter { list[it] is Slot.New }
            when (newItems) {
                "next" -> order.addAll(start, fresh)
                "end" -> order.addAll(fresh.shuffled(random))
                else -> for (i in fresh.shuffled(random)) order.add(start + random.nextInt(order.size - start + 1), i)
            }
            order.toIntArray()
        } else {
            IntArray(list.size) { it }
        }
        return Plan(list, order, currentLeft, added, removed)
    }
}
