package com.moemusicplayer.mediaindex

/** Normalizes only relative M3U paths; absolute paths and URI locators stay unresolved. */
internal fun normalizeSafPlaylistLocator(locator: String): List<String>? {
    val normalized = locator.replace('\\', '/')
    if (normalized.startsWith('/') ||
        normalized.matches(Regex("^[A-Za-z]:.*")) ||
        normalized.matches(Regex("^[A-Za-z][A-Za-z0-9+.-]*:.*"))
    ) return null

    val path = mutableListOf<String>()
    for (component in normalized.split('/')) {
        when (component) {
            "", "." -> Unit
            ".." -> {
                if (path.isEmpty()) throw IllegalArgumentException("relative playlist locator escapes the SAF tree")
                path.removeAt(path.lastIndex)
            }
            else -> path.add(component)
        }
    }
    return path.takeIf { it.isNotEmpty() }
}
