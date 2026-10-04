package app.tauri.nativeaudio

import kotlin.random.Random
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class QueueUpdateTest {
    private fun ids(vararg ids: Long): List<Long?> = ids.map { it }

    /** The plan as old indices (kept / matched) or "new:<item>". */
    private fun describe(plan: QueueUpdate.Plan) = plan.list.map {
        when (it) {
            is QueueUpdate.Slot.Matched -> "old:${it.old}"
            is QueueUpdate.Slot.Kept -> "kept:${it.old}"
            is QueueUpdate.Slot.New -> "new:${it.item}"
        }
    }

    @Test
    fun keepsWhatStaysAddsNewAndRemovesMissing() {
        // A B C D, B plays; the playlist is now A C E B.
        val plan = QueueUpdate.plan(ids(0, 1, 2, 3), List(4) { false }, intArrayOf(0, 1, 2, 3), 1, ids(0, 2, 4, 1), shuffle = false)
        assertEquals(listOf("old:0", "old:2", "new:2", "old:1"), describe(plan))
        assertEquals(false, plan.currentLeft)
    }

    @Test
    fun thePlayingEntryStaysWhenItLeftThePlaylist() {
        val plan = QueueUpdate.plan(ids(0, 1, 2), List(3) { false }, intArrayOf(0, 1, 2), 1, ids(0, 2), shuffle = false)
        assertEquals(listOf("old:0", "kept:1", "old:2"), describe(plan))
        assertTrue(plan.currentLeft)
    }

    @Test
    fun addedEntriesStayAfterTheEntryTheyFollowed() {
        // A X B C (X added after A); the playlist is now C A B.
        val plan = QueueUpdate.plan(ids(0, 9, 1, 2), listOf(false, true, false, false), intArrayOf(0, 1, 2, 3), 0, ids(2, 0, 1), shuffle = false)
        assertEquals(listOf("old:3", "old:0", "kept:1", "old:2"), describe(plan))
    }

    @Test
    fun removedSongsCanStayAndNewOnesGoToTheEnd() {
        // A B C, A plays; the playlist is now N C A: B kept after A, N at the end.
        val plan = QueueUpdate.plan(ids(0, 1, 2), List(3) { false }, intArrayOf(0, 1, 2), 0, ids(9, 2, 0), shuffle = false, keepRemoved = true, newItems = "end")
        assertEquals(listOf("old:2", "old:0", "kept:1", "new:0"), describe(plan))
        assertEquals(false, plan.currentLeft)
        // With shuffle the new ones go at the end of the play order.
        val shuffled = QueueUpdate.plan(ids(0, 1, 2), List(3) { false }, intArrayOf(2, 0, 1), 0, ids(0, 1, 2, 7, 8), shuffle = true, newItems = "end")
        assertEquals(listOf(2, 0, 1), shuffled.order.take(3))
        assertEquals(setOf(3, 4), shuffled.order.drop(3).toSet())
    }

    @Test
    fun newSongsCanPlayNextKeepingTheQueueOrderAndChangesAreCounted() {
        // A B C D, B plays; the playlist is now D C N1 B N2 (A gone): queue order, new ones next.
        val plan = QueueUpdate.plan(ids(0, 1, 2, 3), List(4) { false }, intArrayOf(0, 1, 2, 3), 1, ids(3, 2, 10, 1, 11), shuffle = false, newItems = "next", keepQueueOrder = true)
        assertEquals(listOf("old:1", "new:2", "new:4", "old:2", "old:3"), describe(plan))
        assertEquals(2, plan.added)
        assertEquals(1, plan.removed)
        // Queue order, new ones in place: after the song before them in the playlist.
        val inPlace = QueueUpdate.plan(ids(0, 1, 2), List(3) { false }, intArrayOf(0, 1, 2), 0, ids(9, 2, 10, 0, 1), shuffle = false, keepQueueOrder = true)
        assertEquals(listOf("new:0", "old:0", "old:1", "old:2", "new:2"), describe(inPlace))
        // With shuffle, "next" puts them right after the current entry in play order.
        val shuffled = QueueUpdate.plan(ids(0, 1, 2, 3), List(4) { false }, intArrayOf(3, 1, 0, 2), 1, ids(0, 1, 2, 3, 10, 11), shuffle = true, newItems = "next")
        // List: A B N1 N2 C D (new ones after B there too); old play order D B A C.
        assertEquals(listOf(5, 1, 2, 3, 0, 4), shuffled.order.toList())
    }

    @Test
    fun theSameSongTwiceIsMatchedInOrder() {
        val plan = QueueUpdate.plan(ids(1, 2, 1), List(3) { false }, intArrayOf(0, 1, 2), null, ids(1, 1), shuffle = false)
        assertEquals(listOf("old:0", "old:2"), describe(plan))
    }

    @Test
    fun withShuffleTheOrderStaysAndNewEntriesComeAfterTheCurrent() {
        val oldOrder = intArrayOf(3, 0, 5, 1, 4, 2)
        for (seed in 1..50) {
            // Current: 5 (third in play order). Item 1 removed, item 9 new.
            val plan = QueueUpdate.plan(ids(0, 1, 2, 3, 4, 5), List(6) { false }, oldOrder, 5, ids(0, 2, 3, 4, 5, 9), shuffle = true, random = Random(seed))
            val oldOf = plan.list.map { (it as? QueueUpdate.Slot.Matched)?.old ?: -1 }
            val order = plan.order.map { oldOf[it] }
            assertEquals(listOf(3, 0, 5, 4, 2), order.filter { it >= 0 })
            assertTrue(order.indexOf(-1) > order.indexOf(5))
            assertEquals(6, order.size)
        }
    }
}
