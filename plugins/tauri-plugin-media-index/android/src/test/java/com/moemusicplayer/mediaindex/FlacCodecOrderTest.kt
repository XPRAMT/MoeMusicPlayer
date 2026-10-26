package com.moemusicplayer.mediaindex

import org.junit.Assert.assertEquals
import org.junit.Test

class FlacCodecOrderTest {
    private fun order(mime: String, codecs: List<CodecTraits>) = orderDecodersForMime(mime, codecs) { it }.map { it.name }

    private val qti = CodecTraits("OMX.qti.audio.decoder.flac", softwareOnly = false, hardwareAccelerated = true)
    private val google = CodecTraits("OMX.google.flac.decoder", softwareOnly = true, hardwareAccelerated = false)

    @Test
    fun flacPutsSoftwareDecodersFirstAndKeepsHardwareAsBackup() {
        assertEquals(
            listOf("OMX.google.flac.decoder", "OMX.qti.audio.decoder.flac"),
            order("audio/flac", listOf(qti, google)),
        )
    }

    @Test
    fun softwareIsRecognisedByNameWhenTheFlagIsMissing() {
        val vendor = CodecTraits("c2.qti.flac.decoder", softwareOnly = false, hardwareAccelerated = false)
        val c2 = CodecTraits("c2.android.flac.decoder", softwareOnly = false, hardwareAccelerated = false)
        val omx = CodecTraits("OMX.google.flac.decoder", softwareOnly = false, hardwareAccelerated = false)
        assertEquals(
            listOf("c2.android.flac.decoder", "OMX.google.flac.decoder", "c2.qti.flac.decoder"),
            order("audio/flac", listOf(vendor, c2, omx)),
        )
    }

    @Test
    fun otherMimeTypesKeepTheirOrder() {
        val hw = CodecTraits("OMX.qti.audio.decoder.aac", softwareOnly = false, hardwareAccelerated = true)
        val sw = CodecTraits("OMX.google.aac.decoder", softwareOnly = true, hardwareAccelerated = false)
        assertEquals(
            listOf("OMX.qti.audio.decoder.aac", "OMX.google.aac.decoder"),
            order("audio/mp4a-latm", listOf(hw, sw)),
        )
    }
}
