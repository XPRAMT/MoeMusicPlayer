package com.moemusicplayer.mediaindex

import org.json.JSONArray
import org.json.JSONObject

/**
 * Parser of a USB audio device's raw descriptors (`UsbDeviceConnection.getRawDescriptors()`: the device descriptor
 * followed by every configuration descriptor and what it contains), written from the USB Audio Class 1.0 and 2.0
 * specifications (ADR 0045). Read only and pure: no Android API, so it is unit-tested with byte fixtures.
 *
 * Unknown, truncated or malformed descriptors are skipped; parsing never throws. Sample rates of a UAC2 stream come
 * from its clock source at runtime (a control request), which is not sent: they are null.
 */
object UsbAudioDescriptors {
    private const val DT_DEVICE = 0x01
    private const val DT_CONFIGURATION = 0x02
    private const val DT_INTERFACE = 0x04
    private const val DT_ENDPOINT = 0x05
    private const val CS_INTERFACE = 0x24

    private const val CLASS_AUDIO = 0x01
    private const val SUBCLASS_CONTROL = 0x01
    private const val SUBCLASS_STREAMING = 0x02
    private const val PROTOCOL_UAC2 = 0x20
    private const val PROTOCOL_UAC3 = 0x30

    // Audio Control interface descriptor subtypes (UAC1 4.3.2, UAC2 4.7.2).
    private const val AC_HEADER = 0x01
    private const val AC_INPUT_TERMINAL = 0x02
    private const val AC_OUTPUT_TERMINAL = 0x03
    private const val AC_FEATURE_UNIT = 0x06
    private const val AC2_CLOCK_SOURCE = 0x0A
    private const val AC2_CLOCK_SELECTOR = 0x0B

    // Audio Streaming interface descriptor subtypes.
    private const val AS_GENERAL = 0x01
    private const val AS_FORMAT_TYPE = 0x02

    data class Terminal(
        val id: Int,
        val output: Boolean,
        val terminalType: Int,
        val associatedTerminal: Int,
        /** Output terminal: the unit or terminal it is fed by; null for an input terminal. */
        val sourceId: Int?,
        /** UAC2: the clock entity this terminal runs on. */
        val clockSourceId: Int?,
        /** Input terminal: its channel count. */
        val channels: Int?,
    )

    data class FeatureUnit(
        val id: Int,
        val sourceId: Int,
        /** Channels described after the master (the controls of channel 0 are the master's). */
        val channels: Int,
        val masterMute: Boolean,
        val masterVolume: Boolean,
        val channelMute: Boolean,
        val channelVolume: Boolean,
    )

    data class ClockSource(
        val id: Int,
        /** 0 external, 1 internal fixed, 2 internal variable, 3 internal programmable (bmAttributes D1..0). */
        val clockType: Int,
        val syncedToSof: Boolean,
        /** bmControls D1..0: 0 none, 1 read only, 3 host programmable. */
        val frequencyControl: Int,
        val validityControl: Int,
        val associatedTerminal: Int,
    )

    data class ClockSelector(val id: Int, val inputs: List<Int>)

    data class Endpoint(
        val address: Int,
        /** true: IN (device to host). */
        val input: Boolean,
        /** 0 control, 1 isochronous, 2 bulk, 3 interrupt. */
        val transferType: Int,
        /** Isochronous: 0 none, 1 asynchronous, 2 adaptive, 3 synchronous. */
        val syncType: Int,
        /** Isochronous: 0 data, 1 feedback, 2 implicit feedback data. */
        val usageType: Int,
        val maxPacketSize: Int,
        /** High speed: additional transactions per microframe (wMaxPacketSize bits 12..11). */
        val extraTransactions: Int,
        val interval: Int,
    )

    data class Stream(
        val interfaceNumber: Int,
        val alternateSetting: Int,
        val terminalLink: Int?,
        /** bFormatType: 1 Type I, 2 Type II, 3 Type III; null when not described. */
        val formatType: Int?,
        /** UAC1 wFormatTag, UAC2 bmFormats. */
        val formatBits: Long?,
        val subslotBytes: Int?,
        val bits: Int?,
        val channels: Int?,
        /** UAC1 discrete rates. */
        val rates: List<Int>?,
        /** UAC1 continuous range, [low, high]. */
        val rateRange: List<Int>?,
        val endpoints: List<Endpoint>,
    )

    data class Configuration(
        val value: Int,
        /** 1, 2 or 3 (UAC3: units and formats are not parsed); null when the configuration has no audio control. */
        val uacVersion: Int?,
        val adcVersion: Int?,
        val terminals: List<Terminal>,
        val featureUnits: List<FeatureUnit>,
        val clockSources: List<ClockSource>,
        val clockSelectors: List<ClockSelector>,
        val streams: List<Stream>,
    )

    data class Device(
        val usbVersion: Int?,
        val vendorId: Int?,
        val productId: Int?,
        val configurations: List<Configuration>,
    )

    private class Bytes(private val data: ByteArray, private val start: Int, val length: Int) {
        fun u8(offset: Int): Int? = if (offset in 0 until length) data[start + offset].toInt() and 0xFF else null

        fun u16(offset: Int): Int? {
            val low = u8(offset) ?: return null
            val high = u8(offset + 1) ?: return null
            return low or (high shl 8)
        }

        fun u24(offset: Int): Int? {
            val low = u16(offset) ?: return null
            val high = u8(offset + 2) ?: return null
            return low or (high shl 16)
        }

        fun u32(offset: Int): Long? {
            val low = u16(offset) ?: return null
            val high = u16(offset + 2) ?: return null
            return low.toLong() or (high.toLong() shl 16)
        }
    }

    private class StreamBuilder(val interfaceNumber: Int, val alternateSetting: Int) {
        var terminalLink: Int? = null
        var formatType: Int? = null
        var formatBits: Long? = null
        var subslotBytes: Int? = null
        var bits: Int? = null
        var channels: Int? = null
        var rates: List<Int>? = null
        var rateRange: List<Int>? = null
        val endpoints = mutableListOf<Endpoint>()

        fun build() = Stream(
            interfaceNumber, alternateSetting, terminalLink, formatType, formatBits, subslotBytes, bits, channels,
            rates, rateRange, endpoints.toList(),
        )
    }

    private class ConfigurationBuilder(val value: Int) {
        var uacVersion: Int? = null
        var adcVersion: Int? = null
        val terminals = mutableListOf<Terminal>()
        val featureUnits = mutableListOf<FeatureUnit>()
        val clockSources = mutableListOf<ClockSource>()
        val clockSelectors = mutableListOf<ClockSelector>()
        val streams = mutableListOf<StreamBuilder>()

        fun build() = Configuration(
            value, uacVersion, adcVersion, terminals.toList(), featureUnits.toList(), clockSources.toList(),
            clockSelectors.toList(), streams.map { it.build() },
        )
    }

    /** Parse raw descriptors; never throws (what cannot be read is left out). */
    fun parse(raw: ByteArray?): Device {
        var usbVersion: Int? = null
        var vendorId: Int? = null
        var productId: Int? = null
        val configurations = mutableListOf<ConfigurationBuilder>()
        if (raw == null) return Device(null, null, null, emptyList())
        var configuration: ConfigurationBuilder? = null
        // The interface the following class-specific descriptors and endpoints belong to.
        var role = 0 // 0 none, 1 audio control, 2 audio streaming
        var protocol = 0
        var stream: StreamBuilder? = null
        var offset = 0
        while (offset + 2 <= raw.size) {
            val length = raw[offset].toInt() and 0xFF
            if (length < 2) break // a zero length would never advance: the rest cannot be walked
            val available = minOf(length, raw.size - offset)
            val d = Bytes(raw, offset, available)
            val type = d.u8(1) ?: break
            try {
                when (type) {
                    DT_DEVICE -> {
                        usbVersion = d.u16(2)
                        vendorId = d.u16(8)
                        productId = d.u16(10)
                    }
                    DT_CONFIGURATION -> {
                        configuration = ConfigurationBuilder(d.u8(5) ?: configurations.size + 1).also { configurations += it }
                        role = 0
                        stream = null
                    }
                    DT_INTERFACE -> {
                        val number = d.u8(2)
                        val alternate = d.u8(3)
                        val interfaceClass = d.u8(5)
                        val subclass = d.u8(6)
                        protocol = d.u8(7) ?: 0
                        stream = null
                        role = 0
                        val config = configuration
                        if (config != null && number != null && alternate != null && interfaceClass == CLASS_AUDIO) {
                            when (subclass) {
                                SUBCLASS_CONTROL -> {
                                    role = 1
                                    if (config.uacVersion == null) config.uacVersion = versionOf(protocol, null)
                                }
                                SUBCLASS_STREAMING -> {
                                    role = 2
                                    stream = StreamBuilder(number, alternate).also { config.streams += it }
                                }
                            }
                        }
                    }
                    DT_ENDPOINT -> {
                        val current = stream
                        if (role == 2 && current != null) endpointOf(d)?.let { current.endpoints += it }
                    }
                    CS_INTERFACE -> {
                        val config = configuration
                        if (config != null && role == 1) controlDescriptor(d, protocol, config)
                        val current = stream
                        if (config != null && role == 2 && current != null && config.uacVersion != 3) {
                            streamingDescriptor(d, uac2 = protocol == PROTOCOL_UAC2 || config.uacVersion == 2, current)
                        }
                    }
                }
            } catch (_: RuntimeException) {
                // A descriptor this parser misreads is left out; the walk goes on by bLength.
            }
            offset += length
        }
        return Device(usbVersion, vendorId, productId, configurations.map { it.build() })
    }

    private fun versionOf(protocol: Int, adc: Int?): Int = when {
        protocol == PROTOCOL_UAC3 -> 3
        protocol == PROTOCOL_UAC2 -> 2
        adc != null && adc >= 0x0300 -> 3
        adc != null && adc >= 0x0200 -> 2
        else -> 1
    }

    private fun controlDescriptor(d: Bytes, protocol: Int, config: ConfigurationBuilder) {
        val subtype = d.u8(2) ?: return
        val uac2 = protocol == PROTOCOL_UAC2 || config.uacVersion == 2
        if (protocol == PROTOCOL_UAC3 || config.uacVersion == 3) {
            if (subtype == AC_HEADER) config.adcVersion = d.u16(3) ?: config.adcVersion
            return
        }
        when (subtype) {
            AC_HEADER -> {
                val adc = d.u16(3) ?: return
                config.adcVersion = adc
                config.uacVersion = versionOf(protocol, adc)
            }
            AC_INPUT_TERMINAL -> {
                val id = d.u8(3) ?: return
                config.terminals += Terminal(
                    id = id,
                    output = false,
                    terminalType = d.u16(4) ?: return,
                    associatedTerminal = d.u8(6) ?: 0,
                    sourceId = null,
                    clockSourceId = if (uac2) d.u8(7) else null,
                    channels = if (uac2) d.u8(8) else d.u8(7),
                )
            }
            AC_OUTPUT_TERMINAL -> {
                val id = d.u8(3) ?: return
                config.terminals += Terminal(
                    id = id,
                    output = true,
                    terminalType = d.u16(4) ?: return,
                    associatedTerminal = d.u8(6) ?: 0,
                    sourceId = d.u8(7),
                    clockSourceId = if (uac2) d.u8(8) else null,
                    channels = null,
                )
            }
            AC_FEATURE_UNIT -> featureUnitOf(d, uac2)?.let { config.featureUnits += it }
            AC2_CLOCK_SOURCE -> if (uac2) {
                val id = d.u8(3) ?: return
                val attributes = d.u8(4) ?: return
                val controls = d.u8(5) ?: 0
                config.clockSources += ClockSource(
                    id = id,
                    clockType = attributes and 0x03,
                    syncedToSof = attributes and 0x04 != 0,
                    frequencyControl = controls and 0x03,
                    validityControl = (controls shr 2) and 0x03,
                    associatedTerminal = d.u8(6) ?: 0,
                )
            }
            AC2_CLOCK_SELECTOR -> if (uac2) {
                val id = d.u8(3) ?: return
                val count = d.u8(4) ?: return
                config.clockSelectors += ClockSelector(id, (0 until count).mapNotNull { d.u8(5 + it) })
            }
        }
    }

    /** UAC1 4.3.2.5: bControlSize bytes per channel, D0 mute, D1 volume. UAC2 4.7.2.8: 4 bytes per channel, a 2-bit
     *  field per control (D1..0 mute, D3..2 volume; any non-zero value means the control is present). */
    private fun featureUnitOf(d: Bytes, uac2: Boolean): FeatureUnit? {
        val id = d.u8(3) ?: return null
        val source = d.u8(4) ?: return null
        val controls = mutableListOf<Long>()
        if (uac2) {
            // bLength = 6 + (ch + 1) * 4: the controls start at 5 and end before iFeature.
            var at = 5
            while (at + 4 <= d.length - 1) {
                controls += d.u32(at) ?: break
                at += 4
            }
        } else {
            val size = d.u8(5) ?: return null
            if (size == 0) return null
            var at = 6
            while (at + size <= d.length - 1) {
                var value = 0L
                for (i in 0 until minOf(size, 4)) value = value or ((d.u8(at + i) ?: 0).toLong() shl (8 * i))
                controls += value
                at += size
            }
        }
        if (controls.isEmpty()) return null
        val muteOf = { value: Long -> if (uac2) value and 0x03L != 0L else value and 0x01L != 0L }
        val volumeOf = { value: Long -> if (uac2) value and 0x0CL != 0L else value and 0x02L != 0L }
        val channels = controls.drop(1)
        return FeatureUnit(
            id = id,
            sourceId = source,
            channels = channels.size,
            masterMute = muteOf(controls[0]),
            masterVolume = volumeOf(controls[0]),
            channelMute = channels.any(muteOf),
            channelVolume = channels.any(volumeOf),
        )
    }

    private fun streamingDescriptor(d: Bytes, uac2: Boolean, stream: StreamBuilder) {
        when (d.u8(2)) {
            AS_GENERAL -> {
                stream.terminalLink = d.u8(3)
                if (uac2) {
                    stream.formatType = d.u8(5) ?: stream.formatType
                    stream.formatBits = d.u32(6)
                    stream.channels = d.u8(10) ?: stream.channels
                } else {
                    stream.formatBits = d.u16(5)?.toLong()
                }
            }
            AS_FORMAT_TYPE -> {
                val formatType = d.u8(3) ?: return
                stream.formatType = formatType
                if (formatType != 1 && formatType != 3) return // Type II (encoded at a bit rate) has no sample size
                if (uac2) {
                    stream.subslotBytes = d.u8(4)
                    stream.bits = d.u8(5)
                } else {
                    stream.channels = d.u8(4)
                    stream.subslotBytes = d.u8(5)
                    stream.bits = d.u8(6)
                    val count = d.u8(7) ?: return
                    if (count == 0) {
                        val low = d.u24(8)
                        val high = d.u24(11)
                        if (low != null && high != null) stream.rateRange = listOf(low, high)
                    } else {
                        stream.rates = (0 until count).mapNotNull { d.u24(8 + it * 3) }
                    }
                }
            }
        }
    }

    private fun endpointOf(d: Bytes): Endpoint? {
        val address = d.u8(2) ?: return null
        val attributes = d.u8(3) ?: return null
        val packet = d.u16(4) ?: return null
        return Endpoint(
            address = address,
            input = address and 0x80 != 0,
            transferType = attributes and 0x03,
            syncType = (attributes shr 2) and 0x03,
            usageType = (attributes shr 4) and 0x03,
            maxPacketSize = packet and 0x07FF,
            extraTransactions = (packet shr 11) and 0x03,
            interval = d.u8(6) ?: 0,
        )
    }

    /** The format of a stream, in a word the page turns into text. */
    fun formatName(stream: Stream, uacVersion: Int?): String {
        val bits = stream.formatBits
        if (uacVersion == 2) {
            return when (stream.formatType) {
                1 -> when {
                    bits == null -> "unknown"
                    bits and 0x8000_0000L != 0L -> "raw"
                    bits and 0x01L != 0L -> "pcm"
                    bits and 0x04L != 0L -> "float"
                    bits and 0x02L != 0L -> "pcm8"
                    bits and 0x08L != 0L -> "alaw"
                    bits and 0x10L != 0L -> "mulaw"
                    else -> "unknown"
                }
                2 -> "typeII"
                3 -> "typeIII"
                4 -> "typeIV"
                else -> "unknown"
            }
        }
        return when (bits?.toInt()) {
            0x0001 -> "pcm"
            0x0002 -> "pcm8"
            0x0003 -> "float"
            0x0004 -> "alaw"
            0x0005 -> "mulaw"
            null -> if (stream.formatType == null) "unknown" else typeName(stream.formatType)
            else -> typeName(stream.formatType)
        }
    }

    private fun typeName(formatType: Int?): String = when (formatType) {
        2 -> "typeII"
        3 -> "typeIII"
        else -> "unknown"
    }

    fun hex(raw: ByteArray?): String = raw?.joinToString("") { "%02X".format(it.toInt() and 0xFF) } ?: ""

    /** One configuration as the page reads it (`android_usb_audio_descriptors`). */
    fun configurationJson(config: Configuration): JSONObject {
        val controls = JSONObject()
            .put("adcVersion", config.adcVersion ?: JSONObject.NULL)
            .put("terminals", JSONArray(config.terminals.map { t ->
                JSONObject()
                    .put("id", t.id)
                    .put("direction", if (t.output) "output" else "input")
                    .put("terminalType", t.terminalType)
                    .put("associatedTerminal", t.associatedTerminal)
                    .put("sourceId", t.sourceId ?: JSONObject.NULL)
                    .put("clockSourceId", t.clockSourceId ?: JSONObject.NULL)
                    .put("channels", t.channels ?: JSONObject.NULL)
            }))
            .put("featureUnits", JSONArray(config.featureUnits.map { f ->
                JSONObject()
                    .put("id", f.id)
                    .put("sourceId", f.sourceId)
                    .put("channels", f.channels)
                    .put("masterMute", f.masterMute)
                    .put("masterVolume", f.masterVolume)
                    .put("channelMute", f.channelMute)
                    .put("channelVolume", f.channelVolume)
            }))
            .put("clockSources", JSONArray(config.clockSources.map { c ->
                JSONObject()
                    .put("id", c.id)
                    .put("type", when (c.clockType) {
                        0 -> "external"
                        1 -> "internalFixed"
                        2 -> "internalVariable"
                        else -> "internalProgrammable"
                    })
                    .put("syncedToSof", c.syncedToSof)
                    .put("frequencyControl", c.frequencyControl)
                    .put("validityControl", c.validityControl)
                    .put("associatedTerminal", c.associatedTerminal)
            }))
            .put("clockSelectors", JSONArray(config.clockSelectors.map { c ->
                JSONObject().put("id", c.id).put("inputs", JSONArray(c.inputs))
            }))
        val streams = JSONArray(config.streams.map { s ->
            JSONObject()
                .put("interface", s.interfaceNumber)
                .put("alt", s.alternateSetting)
                .put("terminalLink", s.terminalLink ?: JSONObject.NULL)
                .put("format", formatName(s, config.uacVersion))
                .put("formatType", s.formatType ?: JSONObject.NULL)
                .put("formatBits", s.formatBits ?: JSONObject.NULL)
                .put("subslotBytes", s.subslotBytes ?: JSONObject.NULL)
                .put("bits", s.bits ?: JSONObject.NULL)
                .put("channels", s.channels ?: JSONObject.NULL)
                .put("rates", s.rates?.let { JSONArray(it) } ?: JSONObject.NULL)
                .put("rateRange", s.rateRange?.let { JSONArray(it) } ?: JSONObject.NULL)
                .put("endpoints", JSONArray(s.endpoints.map { e ->
                    JSONObject()
                        .put("address", e.address)
                        .put("direction", if (e.input) "in" else "out")
                        .put("transfer", when (e.transferType) {
                            0 -> "control"
                            1 -> "isochronous"
                            2 -> "bulk"
                            else -> "interrupt"
                        })
                        .put("sync", when (e.syncType) {
                            0 -> "none"
                            1 -> "async"
                            2 -> "adaptive"
                            else -> "sync"
                        })
                        .put("usage", when (e.usageType) {
                            0 -> "data"
                            1 -> "feedback"
                            2 -> "implicitFeedback"
                            else -> "reserved"
                        })
                        .put("maxPacketSize", e.maxPacketSize)
                        .put("extraTransactions", e.extraTransactions)
                        .put("interval", e.interval)
                }))
        })
        return JSONObject()
            .put("configuration", config.value)
            .put("uacVersion", config.uacVersion ?: JSONObject.NULL)
            .put("controls", controls)
            .put("streams", streams)
    }

    /**
     * The parsed device as JSON: the first configuration that has audio at the top level (`uacVersion`, `controls`,
     * `streams`), any other audio configuration under `otherConfigurations`, and the raw bytes as hex.
     */
    fun deviceJson(raw: ByteArray?): JSONObject {
        val device = parse(raw)
        val audio = device.configurations.filter { it.uacVersion != null || it.streams.isNotEmpty() }
        val first = audio.firstOrNull()
        val json = if (first != null) configurationJson(first) else JSONObject()
            .put("configuration", JSONObject.NULL)
            .put("uacVersion", JSONObject.NULL)
            .put("controls", JSONObject.NULL)
            .put("streams", JSONArray())
        return json
            .put("usbVersion", device.usbVersion ?: JSONObject.NULL)
            .put("otherConfigurations", JSONArray(audio.drop(1).map(::configurationJson)))
            .put("raw", hex(raw))
    }
}
