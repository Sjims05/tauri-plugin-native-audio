package app.tauri.nativeaudio

import org.junit.Assert.assertEquals
import org.junit.Test

class TrackedListsTest {
    private fun ids(entries: List<TrackedLists.Entry>) = entries.map { it.id }

    @Test
    fun mergeKeepsTheNewestTimePerIdAndSortsNewestFirst() {
        // This device played pl-1 at 100 and pl-2 at 300; another device played pl-1 at 400 and pl-3 at 200.
        val local = listOf(TrackedLists.Entry("pl-2", 300), TrackedLists.Entry("pl-1", 100))
        val remote = listOf(TrackedLists.Entry("pl-1", 400), TrackedLists.Entry("pl-3", 200))
        val merged = TrackedLists.combine(local + remote, limit = 10)
        assertEquals(listOf("pl-1", "pl-2", "pl-3"), ids(merged))
        assertEquals(400L, merged.first().playedAtMs)
    }

    @Test
    fun mergeIsTheSameWhicheverSideComesFirst() {
        val a = listOf(TrackedLists.Entry(1L, 10), TrackedLists.Entry(2L, 30))
        val b = listOf(TrackedLists.Entry(2L, 20), TrackedLists.Entry(3L, 40))
        assertEquals(ids(TrackedLists.combine(a + b, 10)), ids(TrackedLists.combine(b + a, 10)))
    }

    @Test
    fun mergeCapsAtTheLimit() {
        val entries = (1L..8L).map { TrackedLists.Entry(it, it * 10) }
        assertEquals(listOf(8L, 7L, 6L, 5L, 4L), ids(TrackedLists.combine(entries, limit = 5)))
    }
}
