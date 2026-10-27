package com.moemusicplayer.mediaindex

import com.moemusicplayer.mediaindex.AudioOutputRules.RouteSource
import org.junit.Assert.assertEquals
import org.junit.Test

class RouteSourceTest {
    @Test
    fun aPlayingTrackIsWhereItIsRouted() {
        assertEquals(RouteSource.TRACK, AudioOutputRules.routeSource(trackPlaying = true, hasTrackRoute = true, apiLevel = 36))
        assertEquals(RouteSource.TRACK, AudioOutputRules.routeSource(trackPlaying = true, hasTrackRoute = true, apiLevel = 28))
    }

    @Test
    fun fromAndroid13APausedTrackOrNoTrackUsesTheSystemMediaRoute() {
        // A paused track is not re-routed when a headset connects: its route would be stale.
        assertEquals(RouteSource.SYSTEM, AudioOutputRules.routeSource(trackPlaying = false, hasTrackRoute = true, apiLevel = 33))
        assertEquals(RouteSource.SYSTEM, AudioOutputRules.routeSource(trackPlaying = false, hasTrackRoute = false, apiLevel = 36))
        // Playing but the route is not known yet.
        assertEquals(RouteSource.SYSTEM, AudioOutputRules.routeSource(trackPlaying = true, hasTrackRoute = false, apiLevel = 34))
    }

    @Test
    fun beforeAndroid13ThePausedTracksRouteStays() {
        assertEquals(RouteSource.TRACK, AudioOutputRules.routeSource(trackPlaying = false, hasTrackRoute = true, apiLevel = 32))
        assertEquals(RouteSource.SYSTEM, AudioOutputRules.routeSource(trackPlaying = false, hasTrackRoute = false, apiLevel = 28))
    }
}
