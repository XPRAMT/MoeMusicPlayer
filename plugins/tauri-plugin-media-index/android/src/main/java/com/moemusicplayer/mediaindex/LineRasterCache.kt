package com.moemusicplayer.mediaindex

/** Everything that changes how a rasterised line looks (ADR 0032): a change re-rasterises every line. */
internal data class LineLook(
    val sizePx: Float,
    val color: Int,
    val shadowRadius: Float,
    val shadowColor: Int,
    val strokeWidth: Float,
    val strokeColor: Int,
    val density: Float,
)

/**
 * Lines rasterised once with their shadow / outline (ADR 0032), so a glide only draws a cached image at a new offset
 * and never blurs per frame. Keyed by text under one [LineLook]; a new look drops everything, [retain] drops the lines
 * outside the visible strip window, so memory stays bounded to what can be on screen. Pure (the image type and its
 * creation / release are given), so the invalidation is unit-tested on the JVM.
 */
internal class LineRasterCache<B : Any>(
    private val create: (text: String, look: LineLook) -> B?,
    private val release: (B) -> Unit,
) {
    private var look: LineLook? = null
    private val entries = HashMap<String, B>()

    val size: Int get() = entries.size

    fun setLook(value: LineLook) {
        if (value == look) return
        look = value
        clear()
    }

    /** The image of `text`, rasterised on first use; null when it cannot be (empty, too large): draw directly then. */
    fun get(text: String): B? {
        val current = look ?: return null
        entries[text]?.let { return it }
        val image = create(text, current) ?: return null
        entries[text] = image
        return image
    }

    /** Drop every line not in `keep`. */
    fun retain(keep: Set<String>) {
        val iterator = entries.entries.iterator()
        while (iterator.hasNext()) {
            val entry = iterator.next()
            if (entry.key !in keep) {
                release(entry.value)
                iterator.remove()
            }
        }
    }

    fun clear() {
        entries.values.forEach(release)
        entries.clear()
    }
}
