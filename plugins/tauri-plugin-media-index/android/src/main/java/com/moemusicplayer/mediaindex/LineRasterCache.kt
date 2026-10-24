package com.moemusicplayer.mediaindex

import java.util.concurrent.ConcurrentHashMap

/** Everything that changes how a rasterised line looks (ADR 0032): a change re-rasterises every line. */
internal data class LineLook(
    val sizePx: Float,
    val color: Int,
    val shadowRadius: Float,
    val shadowColor: Int,
    val strokeWidth: Float,
    val strokeColor: Int,
    val density: Float,
    /** Font weight of this line's typeface: lines drawn at another weight are other bitmaps. */
    val weight: Int = FloatingLyricsRules.DEFAULT_FONT_WEIGHT,
    /** The view never left the platform's default face (its weight was never changed from the default): that face, not `weight`'s. */
    val defaultFace: Boolean = false,
)

/**
 * Lines rasterised once with their shadow / outline (ADR 0032), so a glide only draws a cached image at a new offset
 * and never blurs per frame. Keyed by text under one [LineLook]. The images are built by [create] on a worker thread
 * ([submit]) and handed to the main thread ([deliver]) as finished, immutable values; the main thread only ever reads
 * them ([peek]) and never rasterises. A new look bumps [generation]: it drops everything and every result still in
 * flight is discarded when it arrives. [retain] drops the lines outside the wanted window (and cancels their pending
 * work), so memory stays bounded to the window. Pure (the image type, the thread hand-over and the release are given),
 * so the invalidation and eviction are unit-tested on the JVM. Everything but [create] runs on one (the main) thread.
 */
internal class LineRasterCache<B : Any>(
    private val create: (text: String, look: LineLook) -> B?,
    private val release: (B) -> Unit,
    private val submit: (Runnable) -> Unit,
    private val deliver: (Runnable) -> Unit,
    private val onReady: () -> Unit,
) {
    /** A finished line; `image` is null when it cannot be rasterised (empty, too large): draw it directly then. */
    private class Entry<B>(val image: B?)

    private var look: LineLook? = null
    private val entries = HashMap<String, Entry<B>>()
    /** Text -> token of the request in flight; read by the worker to skip work that was cancelled. */
    private val pending = ConcurrentHashMap<String, Int>()
    private var nextToken = 0

    @Volatile
    var generation = 0
        private set

    val size: Int get() = entries.size
    val pendingCount: Int get() = pending.size

    fun setLook(value: LineLook) {
        if (value == look) return
        look = value
        generation++
        clear()
    }

    /** The finished image of `text`, or null when it is not ready or cannot be made (see [isUnavailable]). */
    fun peek(text: String): B? = entries[text]?.image

    /** True when `text` was tried and cannot be an image: the caller draws it directly. */
    fun isUnavailable(text: String): Boolean {
        val entry = entries[text] ?: return false
        return entry.image == null
    }

    /** Ask for `text` to be rasterised in the background; nothing happens when it is there or on its way. */
    fun request(text: String) {
        val current = look ?: return
        if (text.isEmpty() || entries.containsKey(text) || pending.containsKey(text)) return
        val gen = generation
        val token = ++nextToken
        pending[text] = token
        submit(Runnable {
            if (generation != gen || pending[text] != token) return@Runnable
            val image = try {
                create(text, current)
            } catch (_: Throwable) {
                null
            }
            deliver(Runnable { complete(text, gen, token, image) })
        })
    }

    private fun complete(text: String, gen: Int, token: Int, image: B?) {
        if (generation != gen || pending[text] != token) {
            image?.let(release)
            return
        }
        pending.remove(text)
        entries[text] = Entry(image)
        if (image != null) onReady()
    }

    /** Drop every line not in `keep`, finished or not yet started. */
    fun retain(keep: Set<String>) {
        val iterator = entries.entries.iterator()
        while (iterator.hasNext()) {
            val entry = iterator.next()
            if (entry.key !in keep) {
                entry.value.image?.let(release)
                iterator.remove()
            }
        }
        pending.keys.retainAll(keep)
    }

    fun clear() {
        entries.values.forEach { it.image?.let(release) }
        entries.clear()
        pending.clear()
    }
}
