package com.moemusicplayer.mediaindex

import com.moemusicplayer.mediaindex.AudioOutputRules.TYPE_BLE_BROADCAST
import com.moemusicplayer.mediaindex.AudioOutputRules.TYPE_BLE_HEADSET
import com.moemusicplayer.mediaindex.AudioOutputRules.TYPE_BLUETOOTH_A2DP
import com.moemusicplayer.mediaindex.AudioOutputRules.TYPE_BLUETOOTH_SCO
import com.moemusicplayer.mediaindex.AudioOutputRules.TYPE_BUILTIN_EARPIECE
import com.moemusicplayer.mediaindex.AudioOutputRules.TYPE_BUILTIN_SPEAKER
import com.moemusicplayer.mediaindex.AudioOutputRules.TYPE_BUILTIN_SPEAKER_SAFE
import com.moemusicplayer.mediaindex.AudioOutputRules.TYPE_BUS
import com.moemusicplayer.mediaindex.AudioOutputRules.TYPE_REMOTE_SUBMIX
import com.moemusicplayer.mediaindex.AudioOutputRules.TYPE_TELEPHONY
import com.moemusicplayer.mediaindex.AudioOutputRules.TYPE_USB_DEVICE
import com.moemusicplayer.mediaindex.AudioOutputRules.TYPE_USB_HEADSET
import com.moemusicplayer.mediaindex.AudioOutputRules.TYPE_WIRED_HEADPHONES
import com.moemusicplayer.mediaindex.AudioOutputRules.TYPE_WIRED_HEADSET
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The identity, filtering and matching rules of the Android output device choice (ADR 0044). */
class AudioOutputRulesTest {
    private fun device(
        id: Int,
        type: Int,
        name: String = "Device $id",
        address: String = "",
        rates: List<Int> = emptyList(),
        channels: List<Int> = emptyList(),
        encodings: List<Int> = emptyList(),
    ) = OutputDeviceFacts(id, type, name, address, rates, channels, encodings)

    @Test
    fun theListKeepsPlayableDevicesOnlyAndPutsTheSpeakerLast() {
        val all = listOf(
            device(1, TYPE_BUILTIN_EARPIECE, "NX809J"),
            device(2, TYPE_BUILTIN_SPEAKER, "NX809J"),
            device(3, TYPE_BUILTIN_SPEAKER_SAFE, "NX809J"),
            device(4, TYPE_TELEPHONY),
            device(5, TYPE_BLUETOOTH_SCO, "Buds", "AA:BB"),
            device(6, TYPE_BUS),
            device(7, TYPE_REMOTE_SUBMIX),
            device(8, TYPE_USB_HEADSET, "USB DAC"),
            device(9, TYPE_BLUETOOTH_A2DP, "Buds", "AA:BB"),
        )
        assertEquals(listOf(8, 9, 2), AudioOutputRules.userDevices(all).map { it.id })
    }

    @Test
    fun oneDeviceReportedTwiceIsOneRowWithTheUnionOfWhatItTakes() {
        val all = listOf(
            device(10, TYPE_USB_DEVICE, "KA3", rates = listOf(48000, 44100), encodings = listOf(2)),
            device(11, TYPE_USB_HEADSET, "KA3", rates = listOf(96000), encodings = listOf(22)),
        )
        val listed = AudioOutputRules.userDevices(all)
        assertEquals(1, listed.size)
        assertEquals(10, listed[0].id)
        assertEquals(listOf(44100, 48000, 96000), listed[0].sampleRates)
        assertEquals(listOf(2, 22), listed[0].encodings)
        // An empty list means "any", and stays so.
        val any = AudioOutputRules.userDevices(
            listOf(device(12, TYPE_USB_DEVICE, "X", rates = listOf(48000)), device(13, TYPE_USB_HEADSET, "X")),
        )
        assertEquals(emptyList<Int>(), any[0].sampleRates)
    }

    @Test
    fun aLeAudioHeadsetAndItsBroadcastAreOneRow() {
        val listed = AudioOutputRules.userDevices(
            listOf(device(20, TYPE_BLE_HEADSET, "Buds", "11:22"), device(21, TYPE_BLE_BROADCAST, "Buds", "11:22")),
        )
        assertEquals(listOf(20), listed.map { it.id })
    }

    @Test
    fun theIdentityIsKindAndNameAndTheAddressOnlyForBluetooth() {
        assertEquals(
            OutputDevicePreference("usb", "USB DAC", null),
            AudioOutputRules.identityOf(device(1, TYPE_USB_DEVICE, " USB DAC ", "card=1;device=0")),
        )
        assertEquals(
            OutputDevicePreference("bluetooth", "Buds", "AA:BB"),
            AudioOutputRules.identityOf(device(2, TYPE_BLUETOOTH_A2DP, "Buds", "AA:BB")),
        )
        assertEquals(
            OutputDevicePreference("wired", "Headset", null),
            AudioOutputRules.identityOf(device(3, TYPE_WIRED_HEADSET, "Headset")),
        )
        assertNull(AudioOutputRules.identityOf(device(4, TYPE_TELEPHONY)))
    }

    @Test
    fun aDeviceIsFoundAgainAfterItIsPluggedBackInWithANewId() {
        val chosen = AudioOutputRules.identityOf(device(1, TYPE_USB_HEADSET, "KA3"))!!
        assertNull(AudioOutputRules.find(chosen, listOf(device(2, TYPE_BUILTIN_SPEAKER, "NX809J"))))
        val replugged = listOf(device(2, TYPE_BUILTIN_SPEAKER, "NX809J"), device(57, TYPE_USB_DEVICE, "KA3"))
        assertEquals(57, AudioOutputRules.find(chosen, replugged)?.id)
        // The jack reports a headset or headphones depending on the mic: the same choice.
        val wired = AudioOutputRules.identityOf(device(3, TYPE_WIRED_HEADPHONES, "Headphones"))!!
        assertEquals(9, AudioOutputRules.find(wired, listOf(device(9, TYPE_WIRED_HEADSET, "Headphones")))?.id)
        assertNull(AudioOutputRules.find(null, replugged))
    }

    @Test
    fun twoHeadsetsOfOneModelAreToldApartByAddressUnlessOneIsHidden() {
        val mine = OutputDevicePreference("bluetooth", "Buds", "AA:BB")
        assertTrue(AudioOutputRules.matches(mine, device(1, TYPE_BLUETOOTH_A2DP, "Buds", "aa:bb")))
        assertFalse(AudioOutputRules.matches(mine, device(2, TYPE_BLUETOOTH_A2DP, "Buds", "CC:DD")))
        assertTrue(AudioOutputRules.matches(mine, device(3, TYPE_BLUETOOTH_A2DP, "Buds", "")))
        assertFalse(AudioOutputRules.matches(mine, device(4, TYPE_BLUETOOTH_A2DP, "Other", "AA:BB")))
        assertFalse(AudioOutputRules.matches(mine, device(5, TYPE_BLE_HEADSET, "Buds", "AA:BB")))
    }

    @Test
    fun thePreferenceFromRustReadsUnknownDefaultOrADevice() {
        assertEquals(PreferenceReading.Unknown, AudioOutputRules.parsePreference(null))
        assertEquals(PreferenceReading.Known(null), AudioOutputRules.parsePreference("null"))
        assertEquals(PreferenceReading.Known(null), AudioOutputRules.parsePreference("{broken"))
        assertEquals(
            PreferenceReading.Known(OutputDevicePreference("usb", "KA3", null)),
            AudioOutputRules.parsePreference("""{"kind":"usb","productName":"KA3"}"""),
        )
        assertEquals(
            PreferenceReading.Known(OutputDevicePreference("bluetooth", "", "AA:BB")),
            AudioOutputRules.parsePreference("""{"kind":"bluetooth","productName":"","address":"AA:BB"}"""),
        )
        assertEquals(PreferenceReading.Known(null), AudioOutputRules.parsePreference("""{"kind":"usb","productName":" "}"""))
    }

    @Test
    fun theDeviceJsonCarriesWhatThePageStores() {
        val json = AudioOutputRules.deviceJson(device(8, TYPE_BLUETOOTH_A2DP, "Buds", "AA:BB", rates = listOf(48000)))
        assertEquals("bluetooth", json.getString("kind"))
        assertEquals(48000, json.getJSONArray("sampleRates").getInt(0))
        val identity = json.getJSONObject("identity")
        assertEquals("Buds", identity.getString("productName"))
        assertEquals("AA:BB", identity.getString("address"))
        val usb = AudioOutputRules.deviceJson(device(9, TYPE_USB_DEVICE, "KA3", "card=1"))
        assertFalse(usb.getJSONObject("identity").has("address"))
        assertTrue(AudioOutputRules.deviceJson(device(10, TYPE_TELEPHONY)).isNull("identity"))
        assertEquals(
            AudioOutputRules.preferenceOf(usb.getJSONObject("identity")),
            AudioOutputRules.preferenceOf(JSONObject("""{"kind":"usb","productName":"KA3"}""")),
        )
    }

    @Test
    fun theChannelCountOfAMaskIsItsBits() {
        assertEquals(1, AudioOutputRules.channelCountOfMask(4)) // CHANNEL_OUT_MONO
        assertEquals(2, AudioOutputRules.channelCountOfMask(12)) // CHANNEL_OUT_STEREO
        assertEquals(6, AudioOutputRules.channelCountOfMask(252)) // CHANNEL_OUT_5POINT1
    }
}
