package com.moemusicplayer.app

import org.junit.Assert.assertEquals
import org.junit.Test

class AndroidInsetsPolicyTest {
  @Test
  fun rootContentRemainsFullWindowAndGestureAreaDoesNotCreateBottomPadding() {
    val result = AndroidInsetsPolicy.calculate(
      systemBars = AndroidInsetsPolicy.InsetsPx(top = 125, bottom = 28),
      displayCutout = AndroidInsetsPolicy.InsetsPx(top = 18),
      mandatoryGestures = AndroidInsetsPolicy.InsetsPx(bottom = 40),
      tappableElement = AndroidInsetsPolicy.InsetsPx(top = 24),
      ime = AndroidInsetsPolicy.InsetsPx(),
    )

    assertEquals(AndroidInsetsPolicy.InsetsPx.NONE, result.contentPadding)
    assertEquals(125, result.controls.top)
    assertEquals(0, result.controls.bottom)
  }

  @Test
  fun tappableNavigationInsetProtectsControlsWithoutUsingGestureInset() {
    val result = AndroidInsetsPolicy.calculate(
      systemBars = AndroidInsetsPolicy.InsetsPx(top = 80, bottom = 64),
      displayCutout = AndroidInsetsPolicy.InsetsPx(),
      mandatoryGestures = AndroidInsetsPolicy.InsetsPx(bottom = 32),
      tappableElement = AndroidInsetsPolicy.InsetsPx(bottom = 48),
      ime = AndroidInsetsPolicy.InsetsPx(),
    )

    assertEquals(48, result.controls.bottom)
    assertEquals(AndroidInsetsPolicy.InsetsPx.NONE, result.contentPadding)
  }

  @Test
  fun statusBarAndCutoutUseTheLargerTopInsetInsteadOfAddingThem() {
    val result = AndroidInsetsPolicy.calculate(
      systemBars = AndroidInsetsPolicy.InsetsPx(left = 7, top = 30, right = 9, bottom = 22),
      displayCutout = AndroidInsetsPolicy.InsetsPx(left = 12, top = 24, right = 5),
      mandatoryGestures = AndroidInsetsPolicy.InsetsPx(bottom = 15),
      tappableElement = AndroidInsetsPolicy.InsetsPx(top = 4),
      ime = AndroidInsetsPolicy.InsetsPx(),
    )

    assertEquals(30, result.controls.top)
    assertEquals(12, result.controls.left)
    assertEquals(9, result.controls.right)
  }

  @Test
  fun imeRemainsSeparateFromNavigationAndControlInsets() {
    val result = AndroidInsetsPolicy.calculate(
      systemBars = AndroidInsetsPolicy.InsetsPx(top = 24, bottom = 24),
      displayCutout = AndroidInsetsPolicy.InsetsPx(),
      mandatoryGestures = AndroidInsetsPolicy.InsetsPx(bottom = 36),
      tappableElement = AndroidInsetsPolicy.InsetsPx(bottom = 0),
      ime = AndroidInsetsPolicy.InsetsPx(bottom = 310),
    )

    assertEquals(0, result.controls.bottom)
    assertEquals(310, result.imeBottom)
  }
}
