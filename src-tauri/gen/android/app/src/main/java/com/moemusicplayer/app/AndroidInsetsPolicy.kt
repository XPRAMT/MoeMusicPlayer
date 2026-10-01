package com.moemusicplayer.app

/** Converts physical window insets into full-window and interactive-content layout values. */
internal object AndroidInsetsPolicy {
  data class InsetsPx(
    val left: Int = 0,
    val top: Int = 0,
    val right: Int = 0,
    val bottom: Int = 0,
  ) {
    companion object {
      val NONE = InsetsPx()
    }
  }

  data class LayoutInsets(
    val contentPadding: InsetsPx,
    val controls: InsetsPx,
    val imeBottom: Int,
  )

  @Suppress("UNUSED_PARAMETER")
  fun calculate(
    systemBars: InsetsPx,
    displayCutout: InsetsPx,
    mandatoryGestures: InsetsPx,
    tappableElement: InsetsPx,
    ime: InsetsPx,
  ): LayoutInsets {
    return LayoutInsets(
      contentPadding = InsetsPx.NONE,
      controls = InsetsPx(
        left = maxOf(systemBars.left, displayCutout.left, tappableElement.left),
        top = maxOf(systemBars.top, displayCutout.top, tappableElement.top),
        right = maxOf(systemBars.right, displayCutout.right, tappableElement.right),
        bottom = tappableElement.bottom,
      ),
      imeBottom = ime.bottom,
    )
  }
}
