package app.tauri.nativeaudio

import kotlin.random.Random
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ShuffleOrderBuilderTest {
    @Test
    fun ordersArePermutationsStartingWithFirst() {
        repeat(200) { seed ->
            val order = ShuffleOrderBuilder.build(count = 25, first = 7, random = Random(seed))
            assertEquals((0 until 25).toList(), order.sorted())
            assertEquals(7, order.first())
        }
    }

    @Test
    fun recentTracksStayOutOfTheStartOfTheNextPass() {
        repeat(200) { seed ->
            val random = Random(seed)
            val previous = ShuffleOrderBuilder.build(count = 50, first = null, random = random)
            val recent = ShuffleOrderBuilder.recentTail(previous)
            assertEquals(10, recent.size)

            val next = ShuffleOrderBuilder.build(count = 50, first = previous.first(), recent = recent, random = random)
            assertEquals((0 until 50).toList(), next.sorted())
            // Slot 0 is the pinned track; the next recent.size slots must be tracks that weren't recent.
            assertTrue(next.slice(1..recent.size).none { it in recent })
        }
    }

    @Test
    fun sameArtistIsNotPlayedBackToBackWhenAvoidable() {
        // 20 tracks, 4 artists with 5 tracks each.
        val artistOf = { index: Int -> "Artist ${index % 4}" }
        repeat(200) { seed ->
            val order = ShuffleOrderBuilder.build(count = 20, first = 0, artistOf = artistOf, random = Random(seed))
            for (i in 1 until order.size) {
                assertTrue("seed $seed: ${order.toList()}", artistOf(order[i]) != artistOf(order[i - 1]))
            }
        }
    }

    @Test
    fun handlesTinyQueues() {
        assertEquals(0, ShuffleOrderBuilder.build(count = 0, first = null).size)
        assertEquals(listOf(0), ShuffleOrderBuilder.build(count = 1, first = 0).toList())
        assertEquals(emptySet<Int>(), ShuffleOrderBuilder.recentTail(intArrayOf(1, 0)))
    }
}
