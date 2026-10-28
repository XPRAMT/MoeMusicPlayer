package com.moemusicplayer.mediaindex

/**
 * Parsers of the two `dumpsys` texts that carry the real mixer rate of one AudioTrack (ADR 0048). The dumps are plain
 * debugging text, not an API: every parser reads what it needs by name, ignores columns and lines it does not know, and
 * answers null (never a guess) when the track or the rate is not there.
 */
internal object MixerRateDumpParsers {
    /** `dumpsys media.audio_flinger`: the live state of AudioFlinger (the preferred source). */
    fun fromFlingerDump(text: String, portId: Int): Int? {
        val found = LinkedHashSet<Int>()
        var activeRate: Int? = null
        for (section in threadSections(text)) {
            val rate = section.sampleRate ?: continue
            for (row in section.trackRows) {
                if (row.portId != portId) continue
                found.add(rate)
                if (row.active) activeRate = rate
            }
        }
        // An active row wins; otherwise the rows must agree. Rows that disagree are not guessed between.
        return activeRate ?: found.singleOrNull()
    }

    /** `dumpsys media.metrics`: `audio.track.<portId>` (create event, `threadId`) then `audio.thread.<id>` (`readParameters`, `sampleRate`). */
    fun fromMetricsDump(text: String, portId: Int): Int? {
        val records = metricsRecords(text)
        val threadId = records
            .filter { it.kind == "track" && it.id == portId }
            .flatMap { it.events }
            .lastOrNull { it.name == "create" && it.int("threadId") != null }
            ?.int("threadId") ?: return null
        return records
            .filter { it.kind == "thread" && it.id == threadId }
            .flatMap { it.events }
            .lastOrNull { it.name == "readParameters" && it.int("sampleRate") != null }
            ?.int("sampleRate")
            ?.takeIf { it > 0 }
    }

    // ---- flinger dump -------------------------------------------------------------------------------------------

    internal class TrackRow(val portId: Int, val active: Boolean)

    internal class ThreadSection(val name: String, var sampleRate: Int?, val trackRows: MutableList<TrackRow> = ArrayList())

    private val threadHeader = Regex("""^\s*(?:\w+\s+)?[Tt]hread\b.*?\bname\s+([^\s,:]+)""")
    private val sampleRateLine = Regex("""^\s*Sample rate:\s*(\d+)""")
    private val columnNames = listOf("Port Id", "Chn mask", "Main Buf", "Aux Buf", "G db", "L dB", "R dB", "VS dB")

    /**
     * The "Output thread … name AudioOut_xx" sections of the dump with their `Sample rate:` and the rows of their track
     * tables. Only an Output/Playback style thread counts (an input thread has no mixer rate of ours); the "Local log"
     * lines (which repeat removed tracks) and a "Terminated" track table are skipped.
     */
    internal fun threadSections(text: String): List<ThreadSection> {
        val sections = ArrayList<ThreadSection>()
        var current: ThreadSection? = null
        var portColumn = -1
        var activeColumn = -1
        var columnCount = 0
        var typeColumn = false
        var headerIndent = 0
        var inLog = false
        var inTerminated = false
        for (raw in text.lineSequence()) {
            val line = raw.trimEnd()
            val header = if (raw.isNotEmpty() && !raw[0].isWhitespace()) threadHeader.find(line) else null
            if (header != null) {
                val lower = line.lowercase()
                current = if (lower.contains("input thread") || lower.contains("record thread")) null
                else ThreadSection(header.groupValues[1], null).also { sections.add(it) }
                portColumn = -1
                inLog = false
                inTerminated = false
                continue
            }
            val section = current ?: continue
            if (raw.isNotEmpty() && !raw[0].isWhitespace()) {
                // An unindented line that is not a thread header (real dumps put "Bluetooth latency modes are enabled"
                // between a thread's lines) does not end the section; it only ends a track table.
                portColumn = -1
                continue
            }
            val trimmed = line.trim()
            if (trimmed.isEmpty()) {
                portColumn = -1
                continue
            }
            // A line less indented than the table's header ends the table.
            if (portColumn >= 0 && line.length - line.trimStart().length < headerIndent) portColumn = -1
            if (trimmed.startsWith("Local log", ignoreCase = true)) {
                inLog = true
                portColumn = -1
                continue
            }
            if (inLog) continue
            val rate = sampleRateLine.find(line)
            if (rate != null) {
                if (section.sampleRate == null) section.sampleRate = rate.groupValues[1].toIntOrNull()?.takeIf { it > 0 }
                continue
            }
            if (trimmed.contains("erminated", ignoreCase = true) && trimmed.endsWith(":")) {
                inTerminated = true
                portColumn = -1
                continue
            }
            if (trimmed.contains("racks", ignoreCase = true) && trimmed.endsWith(":") && !trimmed.contains("Port Id")) {
                inTerminated = trimmed.contains("erminated", ignoreCase = true)
                portColumn = -1
                continue
            }
            if (trimmed.contains("Port Id")) {
                val columns = splitColumns(trimmed)
                portColumn = columns.indexOf("Port Id")
                activeColumn = columns.indexOf("Active")
                columnCount = columns.size
                typeColumn = columns.firstOrNull() == "Type"
                headerIndent = line.length - line.trimStart().length
                continue
            }
            if (portColumn >= 0 && !inTerminated) {
                // "19779/  10710" (pid/uid) is one column printed as two tokens.
                val tokens = trimmed.replace(Regex("""(\d+/)\s+(\d+)"""), "$1$2").split(WHITESPACE)
                // A plain track has an empty "Type" column, so its row starts at "Id"; a typed row (e.g. "F") starts at "Type".
                val shift = if (typeColumn && tokens.first().toIntOrNull() != null) 1 else 0
                val port = tokens.getOrNull(portColumn - shift)?.toIntOrNull()
                // A track row carries about as many tokens as the header (a few may be missing at the end on some ROMs);
                // a stray line with a number in that place is not taken for one.
                if (port != null && tokens.size >= (columnCount - 4 - shift).coerceAtLeast(portColumn - shift + 1)) {
                    section.trackRows.add(TrackRow(port, isActive(tokens, activeColumn - shift)))
                }
            }
        }
        return sections
    }

    private val WHITESPACE = Regex("""\s+""")

    /** The header's column names, the known two-word names folded into one. */
    private fun splitColumns(header: String): List<String> {
        var rest = header
        val folded = HashMap<String, String>()
        for ((index, name) in columnNames.withIndex()) {
            val token = "\u0001$index\u0001"
            if (rest.contains(name)) {
                rest = rest.replace(name, token)
                folded[token] = name
            }
        }
        return rest.trim().split(WHITESPACE).map { folded[it] ?: it }
    }

    /** The row's "Active" column (named by the header) is "yes"; without that column, the first yes/no token. */
    private fun isActive(tokens: List<String>, activeColumn: Int): Boolean {
        val flag = tokens.getOrNull(activeColumn)?.takeIf { activeColumn >= 0 }
            ?: tokens.take(4).firstOrNull { it.equals("yes", true) || it.equals("no", true) }
        return flag.equals("yes", true)
    }

    // ---- media.metrics dump -------------------------------------------------------------------------------------

    internal class MetricsEvent(val name: String, private val fields: Map<String, String>) {
        fun int(key: String): Int? = fields[key]?.toIntOrNull()
    }

    internal class MetricsRecord(val kind: String, val id: Int, val events: MutableList<MetricsEvent> = ArrayList())

    private val recordKey = Regex("""audio\.(track|thread)\.(\d+)""")
    private val keyValue = Regex("""(?<![\w.])([A-Za-z][\w.]*)=([^\s,;\])}]+)""")

    /**
     * The `audio.track.<id>` and `audio.thread.<id>` records: a record starts at a line that names it and runs to the
     * next such line; its events start at `event#=<name>` and carry `key=value` pairs (on the same or the next lines).
     */
    internal fun metricsRecords(text: String): List<MetricsRecord> {
        val records = ArrayList<MetricsRecord>()
        val body = ArrayList<String>()
        var current: MetricsRecord? = null
        fun flush() {
            val record = current ?: return
            record.events.addAll(eventsOf(body.joinToString("\n")))
            body.clear()
        }
        for (line in text.lineSequence()) {
            val key = recordKey.find(line)
            if (key != null) {
                flush()
                current = MetricsRecord(key.groupValues[1], key.groupValues[2].toInt()).also { records.add(it) }
                body.add(line.substring(key.range.last + 1))
                continue
            }
            if (line.isBlank()) {
                flush()
                current = null
                continue
            }
            if (current != null) body.add(line)
        }
        flush()
        return records
    }

    private fun eventsOf(body: String): List<MetricsEvent> {
        val events = ArrayList<MetricsEvent>()
        val parts = Regex("""event#=""").split(body)
        for (part in parts.drop(1)) {
            val name = part.takeWhile { !it.isWhitespace() && it != ',' && it != ';' }
            val fields = HashMap<String, String>()
            for (match in keyValue.findAll(part)) fields.putIfAbsent(match.groupValues[1], match.groupValues[2])
            events.add(MetricsEvent(name, fields))
        }
        return events
    }
}
