package com.moemusicplayer.mediaindex

import androidx.media3.common.MimeTypes
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.mediacodec.MediaCodecInfo
import androidx.media3.exoplayer.mediacodec.MediaCodecSelector

/** What the ordering needs to know about one decoder. */
data class CodecTraits(val name: String, val softwareOnly: Boolean, val hardwareAccelerated: Boolean)

/**
 * For FLAC, software decoders come first and hardware decoders stay after them as backups; every other MIME type
 * keeps the order it was given. Vendor FLAC decoders (for example OMX.qti.audio.decoder.flac on a Sony Xperia
 * running Android 9) configure fine and then fail mid-decode, which Media3's decoder fallback cannot catch.
 */
fun <T> orderDecodersForMime(mimeType: String, codecs: List<T>, traits: (T) -> CodecTraits): List<T> {
    if (mimeType != MimeTypes.AUDIO_FLAC) return codecs
    // sortedBy is stable: the relative order inside each group is unchanged.
    return codecs.sortedBy { if (isSoftwareDecoder(traits(it))) 0 else 1 }
}

fun isSoftwareDecoder(codec: CodecTraits): Boolean {
    val name = codec.name.lowercase()
    return codec.softwareOnly ||
        name.startsWith("omx.google.") ||
        name.startsWith("c2.android.") ||
        name.startsWith("c2.google.")
}

/** [MediaCodecSelector.DEFAULT] with FLAC reordered by [orderDecodersForMime]. */
@UnstableApi
object SoftwareFirstFlacCodecSelector : MediaCodecSelector {
    override fun getDecoderInfos(
        mimeType: String,
        requiresSecureDecoder: Boolean,
        requiresTunnelingDecoder: Boolean,
    ): List<MediaCodecInfo> = orderDecodersForMime(
        mimeType,
        MediaCodecSelector.DEFAULT.getDecoderInfos(mimeType, requiresSecureDecoder, requiresTunnelingDecoder),
    ) { CodecTraits(it.name, it.softwareOnly, it.hardwareAccelerated) }
}
