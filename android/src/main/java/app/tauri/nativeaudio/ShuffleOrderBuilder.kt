package app.tauri.nativeaudio

import kotlin.math.min
import kotlin.random.Random

/**
 * Builds shuffled play orders (permutations of queue indices) that feel random to a listener:
 * - every track plays once per pass,
 * - tracks from the end of the previous pass are kept out of the start of the next one,
 * - where possible, two tracks by the same artist don't play back to back.
 */
object ShuffleOrderBuilder {
    /**
     * @param count queue length.
     * @param first index to play first (the current track), or null.
     * @param recent indices played at the end of the previous pass; they are kept out of the
     *   first `recent.size` slots after [first].
     * @param artistOf artist of each index, used to spread artists apart.
     */
    fun build(
        count: Int,
        first: Int?,
        recent: Set<Int> = emptySet(),
        artistOf: (Int) -> String? = { null },
        random: Random = Random.Default,
    ): IntArray {
        if (count <= 0) return IntArray(0)
        val pinned = first?.takeIf { it in 0 until count }
        val rest = (0 until count).filter { it != pinned }
        val fresh = rest.filter { it !in recent }.shuffled(random)
        val stale = rest.filter { it in recent }

        val head = fresh.take(min(fresh.size, stale.size)).toMutableList()
        val tail = (fresh.drop(head.size) + stale).shuffled(random).toMutableList()

        val artists = HashMap<Int, String?>()
        val normalizedArtist = { index: Int ->
            artists.getOrPut(index) { artistOf(index)?.trim()?.lowercase()?.takeIf { it.isNotEmpty() } }
        }
        val pinnedArtist = pinned?.let(normalizedArtist)
        spreadArtists(head, pinnedArtist, normalizedArtist)
        spreadArtists(tail, head.lastOrNull()?.let(normalizedArtist) ?: pinnedArtist, normalizedArtist)

        return (listOfNotNull(pinned) + head + tail).toIntArray()
    }

    /** The last ~20% of [order]: what the next pass should not start with. */
    fun recentTail(order: IntArray): Set<Int> {
        if (order.size <= 2) return emptySet()
        return order.takeLast(maxOf(1, order.size / 5)).toSet()
    }

    /**
     * Reorders the already shuffled [items] so no two neighbours share an artist, when that's possible.
     * Walks the shuffled list and takes the first track whose artist differs from the previous one,
     * except when one artist fills more than half of what's left: that artist has to go now, or its
     * tracks would end up back to back at the end.
     */
    private fun spreadArtists(items: MutableList<Int>, previousArtist: String?, artistOf: (Int) -> String?) {
        val remaining = items.toMutableList()
        val counts = HashMap<String, Int>()
        for (index in remaining) artistOf(index)?.let { counts[it] = (counts[it] ?: 0) + 1 }

        items.clear()
        var previous = previousArtist
        while (remaining.isNotEmpty()) {
            val left = remaining.size
            val forced = counts.entries.firstOrNull { it.key != previous && it.value * 2 > left }?.key
            val pick = if (forced != null) {
                remaining.indexOfFirst { artistOf(it) == forced }
            } else {
                remaining.indexOfFirst { previous == null || artistOf(it) != previous }.takeIf { it >= 0 } ?: 0
            }
            val index = remaining.removeAt(pick)
            val artist = artistOf(index)
            if (artist != null) counts[artist] = counts.getValue(artist) - 1
            items.add(index)
            previous = artist
        }
    }
}
