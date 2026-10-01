package app.tauri.nativeaudio

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test

class QueueOrderTest {
    // List: [A, B, C, D] = indices 0..3, shuffled play order C, A, D, B.
    private val order = intArrayOf(2, 0, 3, 1)

    @Test
    fun playNextGoesRightAfterTheCurrentTrackInPlayOrder() {
        // Current track is A (index 0, play position 1). "Play next" inserts X into the list after
        // A (list index 1) and into the play order right after A.
        val result = QueueOrder.insert(order, at = 1, count = 1, playPosition = 2)
        // List is now [A, X, B, C, D]: C=3, A=0, X=1, D=4, B=2.
        assertArrayEquals(intArrayOf(3, 0, 1, 4, 2), result)
    }

    @Test
    fun addToEndGoesToTheEndOfThePlayOrder() {
        val result = QueueOrder.insert(order, at = 4, count = 2, playPosition = order.size)
        assertArrayEquals(intArrayOf(2, 0, 3, 1, 4, 5), result)
    }

    @Test
    fun removeKeepsTheRestOfTheOrder() {
        // Remove B (index 1): list [A, C, D] = 0, 1, 2. Order C, A, D.
        assertArrayEquals(intArrayOf(1, 0, 2), QueueOrder.remove(order, at = 1))
    }

    @Test
    fun moveChangesListPositionsButNotPlayOrder() {
        // Move A (0) to the end: list [B, C, D, A] = B0 C1 D2 A3. Order still C, A, D, B.
        assertArrayEquals(intArrayOf(1, 3, 2, 0), QueueOrder.move(order, from = 0, to = 3))
        // And back to the front.
        assertArrayEquals(order, QueueOrder.move(intArrayOf(1, 3, 2, 0), from = 3, to = 0))
    }

    @Test
    fun movedIndexMatchesAListMove() {
        val list = mutableListOf("A", "B", "C", "D", "E")
        for (from in list.indices) for (to in list.indices) {
            val moved = list.toMutableList().apply { add(to, removeAt(from)) }
            for (i in list.indices) assertEquals(list[i], moved[QueueOrder.movedIndex(i, from, to)])
        }
    }
}
