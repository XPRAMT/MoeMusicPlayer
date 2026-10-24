package com.moemusicplayer.mediaindex

import android.graphics.Bitmap
import android.opengl.EGL14
import android.opengl.EGLConfig
import android.opengl.EGLContext
import android.opengl.EGLDisplay
import android.opengl.EGLSurface
import android.opengl.GLES20
import android.opengl.GLUtils
import android.util.Log
import android.view.Surface
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.ConcurrentHashMap

private const val TAG = "MoeMediaIndex"

/** EGL / GL could not be set up (no display, no config, no context, no surface, a shader that does not build). */
internal class GlInitException(message: String, cause: Throwable? = null) : RuntimeException(message, cause)

/** The EGL context was lost (the GPU reset, the process lost its context): everything GL must be rebuilt. */
internal class GlContextLostException : RuntimeException("EGL context lost")

/** The window surface is gone (destroyed under the renderer); frames stop until a new surface is attached. */
internal class GlSurfaceLostException : RuntimeException("EGL surface lost")

/** One texture-backed line image: `key` = which row, which text, in the text colour or the accent. */
internal data class TexKey(val row: Int, val text: String, val accent: Boolean)

/** The tiles (GL texture names) of one finished line; `textures` is empty for a line that cannot be an image. */
internal class GlLine(
    val textures: IntArray,
    val tiles: List<TilePlan.Tile>,
    val imageW: Int,
    val imageH: Int,
    val pad: Int,
    val bytes: Long,
)

/**
 * The line textures (ADR 0032, GL overlay). Everything but the raster jobs runs on the render thread: a line is
 * requested, rasterised on the shared low-priority worker ([RasterWorker], one bitmap per tile), handed to the render
 * thread, uploaded a few tiles per frame ([pump]) and recycled; a look change (generation) drops everything and discards
 * results still in flight; [retain] keeps only the wanted lines (and cancels the rest of the queue); a byte cap with
 * least-recently-used eviction is the safety net. A line is drawable only when all its tiles are uploaded.
 */
internal class TextureStore(
    private val postToRender: (Runnable) -> Unit,
    private val onActivity: () -> Unit,
) {
    private class Partial(val key: TexKey, val set: TileSet, val textures: IntArray, var next: Int)

    private val lines = HashMap<TexKey, GlLine>()
    private val pending = ConcurrentHashMap<TexKey, Int>()
    private val lru = LruBytes<TexKey>()
    private val queue = ArrayDeque<Partial>()
    private var generation = -1
    private var nextToken = 0
    private var mainLook: LineLook? = null
    private var secondLook: LineLook? = null
    private var accentLook: LineLook? = null
    private var chunk = TilePlan.chunkFor(2048)
    private var keep: Set<TexKey> = emptySet()

    val textureCount: Int get() = lines.size
    val textureBytes: Long get() = lru.totalBytes

    /** Work is on its way (a raster job or an upload): another frame is wanted when it lands. */
    fun hasPendingWork(): Boolean = pending.isNotEmpty() || queue.isNotEmpty()

    fun setChunk(maxTextureSize: Int) {
        chunk = TilePlan.chunkFor(maxTextureSize)
    }

    /** A new raster look generation drops every texture and cancels all work; the same generation changes nothing. */
    fun setLooks(look: RenderLook, glValid: Boolean) {
        if (look.generation == generation) return
        clear(glValid)
        generation = look.generation
        mainLook = look.mainLook
        secondLook = look.secondLook
        accentLook = look.accentLook
    }

    fun get(key: TexKey): GlLine? {
        val line = lines[key] ?: return null
        lru.touch(key)
        return line
    }

    /** Ask for `key` to be rasterised in the background; nothing happens when it is there or on its way. */
    fun request(key: TexKey) {
        if (key.text.isEmpty() || lines.containsKey(key) || pending.containsKey(key)) return
        val look = (if (key.row == 0) (if (key.accent) accentLook else mainLook) else secondLook) ?: return
        val gen = generation
        val token = ++nextToken
        val tileChunk = chunk
        pending[key] = token
        RasterWorker.submit(Runnable {
            if (pending[key] != token) return@Runnable
            val set = try {
                LineRasteriser.renderTiles(key.text, look, tileChunk)
            } catch (_: Throwable) {
                null
            }
            postToRender(Runnable { deliver(key, gen, token, set) })
        })
    }

    private fun deliver(key: TexKey, gen: Int, token: Int, set: TileSet?) {
        if (generation != gen || pending[key] != token) {
            set?.bitmaps?.forEach { it.recycle() }
            return
        }
        pending.remove(key)
        if (set == null) {
            // Cannot be an image (too large): remembered, so it is not asked for again.
            lines[key] = GlLine(IntArray(0), emptyList(), 0, 0, 0, 0)
        } else {
            queue.addLast(Partial(key, set, IntArray(set.bitmaps.size), 0))
        }
        onActivity()
    }

    /**
     * Uploads finished tiles, at least one and about `budgetBytes` per call so a long line does not stall a frame.
     * True when something was uploaded (the picture may change). GL thread.
     */
    fun pump(budgetBytes: Int): Boolean {
        var spent = 0
        var uploaded = false
        while (queue.isNotEmpty()) {
            val partial = queue.first()
            val bitmap = partial.set.bitmaps[partial.next]
            val bytes = bitmap.width * bitmap.height * 4
            if (spent > 0 && spent + bytes > budgetBytes) break
            partial.textures[partial.next] = upload(bitmap)
            if (LyricsTrace.on) LyricsTrace.glUploaded(bytes)
            partial.next++
            spent += bytes
            uploaded = true
            if (partial.next == partial.textures.size) {
                queue.removeFirst()
                val set = partial.set
                val total = set.imageW.toLong() * set.imageH * 4
                lines[partial.key] = GlLine(partial.textures, set.tiles, set.imageW, set.imageH, set.pad, total)
                lru.put(partial.key, total)
            }
        }
        if (uploaded) {
            evict()
            if (LyricsTrace.on) LyricsTrace.glTextures(lines.size, lru.totalBytes)
        }
        return uploaded
    }

    private fun upload(bitmap: Bitmap): Int {
        val ids = IntArray(1)
        GLES20.glGenTextures(1, ids, 0)
        GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, ids[0])
        GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_MIN_FILTER, GLES20.GL_LINEAR)
        GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_MAG_FILTER, GLES20.GL_LINEAR)
        GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_WRAP_S, GLES20.GL_CLAMP_TO_EDGE)
        GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_WRAP_T, GLES20.GL_CLAMP_TO_EDGE)
        try {
            GLUtils.texImage2D(GLES20.GL_TEXTURE_2D, 0, bitmap, 0)
        } finally {
            bitmap.recycle()
        }
        if (LyricsTrace.on) {
            val error = GLES20.glGetError()
            if (error != GLES20.GL_NO_ERROR) Log.w(TAG, "lyrics gl: texImage2D error 0x${Integer.toHexString(error)}")
        }
        return ids[0]
    }

    private val deleteBuffer = IntArray(1)

    private fun deleteTexture(id: Int, glValid: Boolean) {
        if (!glValid || id == 0) return
        deleteBuffer[0] = id
        GLES20.glDeleteTextures(1, deleteBuffer, 0)
    }

    private fun evict() {
        if (lru.totalBytes <= CAP_BYTES) return
        for (key in lru.evict(CAP_BYTES) { it in keep }) {
            lines.remove(key)?.let { line -> line.textures.forEach { deleteTexture(it, true) } }
        }
    }

    /** Keep only `wanted`: the other lines are deleted and their pending work (queued uploads included) is cancelled. */
    fun retain(wanted: Set<TexKey>) {
        keep = wanted
        val iterator = lines.entries.iterator()
        while (iterator.hasNext()) {
            val entry = iterator.next()
            if (entry.key !in wanted) {
                entry.value.textures.forEach { deleteTexture(it, true) }
                lru.remove(entry.key)
                iterator.remove()
            }
        }
        pending.keys.retainAll(wanted)
        val queued = queue.iterator()
        while (queued.hasNext()) {
            val partial = queued.next()
            if (partial.key in wanted) continue
            for (i in partial.set.bitmaps.indices) {
                if (i < partial.next) deleteTexture(partial.textures[i], true) else partial.set.bitmaps[i].recycle()
            }
            queued.remove()
        }
        if (LyricsTrace.on) LyricsTrace.glTextures(lines.size, lru.totalBytes)
    }

    /** Forget everything. `glValid` false when the context is gone (its textures went with it). */
    fun clear(glValid: Boolean) {
        for (line in lines.values) line.textures.forEach { deleteTexture(it, glValid) }
        lines.clear()
        lru.clear()
        pending.clear()
        for (partial in queue) {
            for (i in partial.set.bitmaps.indices) {
                if (i < partial.next) deleteTexture(partial.textures[i], glValid) else partial.set.bitmaps[i].recycle()
            }
        }
        queue.clear()
        keep = emptySet()
        generation = -1
        if (LyricsTrace.on) LyricsTrace.glTextures(0, 0L)
    }

    companion object {
        /** Soft cap of texture memory: lines outside the wanted set are dropped oldest first beyond it. */
        const val CAP_BYTES = 48L * 1024 * 1024
    }
}

/**
 * OpenGL ES 2.0 drawing of the floating lyrics (ADR 0032): EGL context and window surface on the render thread, one
 * clear and one draw per frame. Each line is a premultiplied-alpha texture (tiles for wide lines); the fragment shader
 * samples it at a sub-pixel position with linear filtering and multiplies by a mask alpha computed from six gradient
 * stops (the edge fade of the "none" look, or the continuous look's centre mask), and for a karaoke line mixes the
 * text-colour and accent images at the edge. The move-mode background is drawn first by a rounded-rectangle shader.
 * All methods run on the render thread.
 */
internal class GlLyricsRenderer(
    postToRender: (Runnable) -> Unit,
    onActivity: () -> Unit,
) {
    val store = TextureStore(postToRender, onActivity)

    private var display: EGLDisplay = EGL14.EGL_NO_DISPLAY
    private var context: EGLContext = EGL14.EGL_NO_CONTEXT
    private var config: EGLConfig? = null
    private var eglSurface: EGLSurface = EGL14.EGL_NO_SURFACE
    private var resourcesReady = false

    private var textProgram = 0
    private var textPos = 0
    private var textUv = 0
    private var textScale = 0
    private var textAreaLeft = 0
    private var textKaraoke = 0
    private var textEdge = 0
    private var textStopX = 0
    private var textStopV = 0
    private var bgProgram = 0
    private var bgPos = 0
    private var bgScale = 0
    private var bgRect = 0
    private var bgRadius = 0
    private var bgColor = 0
    private val vertices = ByteBuffer.allocateDirect(16 * 4).order(ByteOrder.nativeOrder()).asFloatBuffer()
    private val sizeBuffer = IntArray(1)

    val hasSurface: Boolean get() = eglSurface != EGL14.EGL_NO_SURFACE

    // ---- EGL

    private fun createContext() {
        display = EGL14.eglGetDisplay(EGL14.EGL_DEFAULT_DISPLAY)
        if (display == EGL14.EGL_NO_DISPLAY) throw GlInitException("eglGetDisplay failed")
        val version = IntArray(2)
        if (!EGL14.eglInitialize(display, version, 0, version, 1)) throw GlInitException("eglInitialize failed 0x${Integer.toHexString(EGL14.eglGetError())}")
        val attributes = intArrayOf(
            EGL14.EGL_RENDERABLE_TYPE, EGL14.EGL_OPENGL_ES2_BIT,
            EGL14.EGL_SURFACE_TYPE, EGL14.EGL_WINDOW_BIT,
            EGL14.EGL_RED_SIZE, 8, EGL14.EGL_GREEN_SIZE, 8, EGL14.EGL_BLUE_SIZE, 8, EGL14.EGL_ALPHA_SIZE, 8,
            EGL14.EGL_DEPTH_SIZE, 0, EGL14.EGL_STENCIL_SIZE, 0,
            EGL14.EGL_NONE,
        )
        val configs = arrayOfNulls<EGLConfig>(1)
        val count = IntArray(1)
        if (!EGL14.eglChooseConfig(display, attributes, 0, configs, 0, 1, count, 0) || count[0] <= 0 || configs[0] == null) {
            throw GlInitException("no RGBA8888 ES2 window config")
        }
        config = configs[0]
        context = EGL14.eglCreateContext(display, config, EGL14.EGL_NO_CONTEXT, intArrayOf(EGL14.EGL_CONTEXT_CLIENT_VERSION, 2, EGL14.EGL_NONE), 0)
        if (context == EGL14.EGL_NO_CONTEXT) throw GlInitException("eglCreateContext failed 0x${Integer.toHexString(EGL14.eglGetError())}")
    }

    /** Creates the window surface for `surface`, makes the context current and builds the shaders on first use. */
    fun attachSurface(surface: Surface) {
        if (context == EGL14.EGL_NO_CONTEXT) createContext()
        destroySurface()
        eglSurface = EGL14.eglCreateWindowSurface(display, config, surface, intArrayOf(EGL14.EGL_NONE), 0)
        if (eglSurface == EGL14.EGL_NO_SURFACE) throw GlInitException("eglCreateWindowSurface failed 0x${Integer.toHexString(EGL14.eglGetError())}")
        if (!EGL14.eglMakeCurrent(display, eglSurface, eglSurface, context)) {
            val error = EGL14.eglGetError()
            destroySurface()
            if (error == EGL14.EGL_CONTEXT_LOST) throw GlContextLostException()
            throw GlInitException("eglMakeCurrent failed 0x${Integer.toHexString(error)}")
        }
        if (!resourcesReady) buildResources()
        EGL14.eglSwapInterval(display, 1)
    }

    /** Releases the window surface (the context and its textures stay). */
    fun destroySurface() {
        if (display != EGL14.EGL_NO_DISPLAY) {
            EGL14.eglMakeCurrent(display, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_CONTEXT)
            if (eglSurface != EGL14.EGL_NO_SURFACE) EGL14.eglDestroySurface(display, eglSurface)
        }
        eglSurface = EGL14.EGL_NO_SURFACE
    }

    /** The context is gone or being replaced: forget every GL object (they are not deleted, the context owned them). */
    fun dropContext() {
        store.clear(false)
        resourcesReady = false
        textProgram = 0
        bgProgram = 0
        destroySurface()
        if (display != EGL14.EGL_NO_DISPLAY && context != EGL14.EGL_NO_CONTEXT) EGL14.eglDestroyContext(display, context)
        context = EGL14.EGL_NO_CONTEXT
        config = null
    }

    /** Everything, for good (the panel is gone). */
    fun release() {
        dropContext()
        if (display != EGL14.EGL_NO_DISPLAY) EGL14.eglTerminate(display)
        display = EGL14.EGL_NO_DISPLAY
    }

    /** The surface's size in px as EGL sees it (it follows the window); null when there is no surface. */
    fun surfaceSize(out: IntArray): Boolean {
        if (eglSurface == EGL14.EGL_NO_SURFACE) return false
        if (!EGL14.eglQuerySurface(display, eglSurface, EGL14.EGL_WIDTH, sizeBuffer, 0)) return false
        out[0] = sizeBuffer[0]
        if (!EGL14.eglQuerySurface(display, eglSurface, EGL14.EGL_HEIGHT, sizeBuffer, 0)) return false
        out[1] = sizeBuffer[0]
        return out[0] > 0 && out[1] > 0
    }

    /** Presents the frame. Throws when the context or the surface is lost. */
    fun swap() {
        if (EGL14.eglSwapBuffers(display, eglSurface)) return
        when (EGL14.eglGetError()) {
            EGL14.EGL_CONTEXT_LOST -> throw GlContextLostException()
            EGL14.EGL_BAD_SURFACE, EGL14.EGL_BAD_NATIVE_WINDOW, EGL14.EGL_BAD_ALLOC -> throw GlSurfaceLostException()
            else -> throw GlSurfaceLostException()
        }
    }

    // ---- shaders

    private fun buildResources() {
        val max = IntArray(1)
        GLES20.glGetIntegerv(GLES20.GL_MAX_TEXTURE_SIZE, max, 0)
        store.setChunk(if (max[0] > 0) max[0] else 2048)
        val textVertex = """
            attribute vec2 aPos;
            attribute vec2 aUv;
            uniform vec2 uScale;
            uniform float uAreaLeft;
            varying vec2 vUv;
            varying float vArea;
            void main() {
              gl_Position = vec4(aPos.x * uScale.x - 1.0, aPos.y * uScale.y + 1.0, 0.0, 1.0);
              vUv = aUv;
              vArea = aPos.x - uAreaLeft;
            }
        """.trimIndent()
        val textFragment = """
            #ifdef GL_FRAGMENT_PRECISION_HIGH
            precision highp float;
            #else
            precision mediump float;
            #endif
            uniform sampler2D uTex0;
            uniform sampler2D uTex1;
            uniform float uKaraoke;
            uniform float uEdge;
            uniform float uStopX[6];
            uniform float uStopV[6];
            varying vec2 vUv;
            varying float vArea;
            float maskAt(float x) {
              float r = uStopV[5];
              float done = 0.0;
              for (int i = 0; i < 5; i++) {
                if (done < 0.5 && x <= uStopX[i + 1]) {
                  float a = uStopX[i];
                  float b = uStopX[i + 1];
                  r = b > a ? mix(uStopV[i], uStopV[i + 1], clamp((x - a) / (b - a), 0.0, 1.0)) : uStopV[i + 1];
                  done = 1.0;
                }
              }
              if (x < 0.0 || x > uStopX[5]) r = 0.0;
              return r;
            }
            void main() {
              vec4 c = texture2D(uTex0, vUv);
              if (uKaraoke > 0.5) {
                vec4 a = texture2D(uTex1, vUv);
                c = mix(c, a, clamp(uEdge - vArea + 0.5, 0.0, 1.0));
              }
              gl_FragColor = c * maskAt(vArea);
            }
        """.trimIndent()
        textProgram = link(textVertex, textFragment)
        textPos = GLES20.glGetAttribLocation(textProgram, "aPos")
        textUv = GLES20.glGetAttribLocation(textProgram, "aUv")
        textScale = GLES20.glGetUniformLocation(textProgram, "uScale")
        textAreaLeft = GLES20.glGetUniformLocation(textProgram, "uAreaLeft")
        textKaraoke = GLES20.glGetUniformLocation(textProgram, "uKaraoke")
        textEdge = GLES20.glGetUniformLocation(textProgram, "uEdge")
        textStopX = GLES20.glGetUniformLocation(textProgram, "uStopX[0]")
        textStopV = GLES20.glGetUniformLocation(textProgram, "uStopV[0]")
        GLES20.glUseProgram(textProgram)
        GLES20.glUniform1i(GLES20.glGetUniformLocation(textProgram, "uTex0"), 0)
        GLES20.glUniform1i(GLES20.glGetUniformLocation(textProgram, "uTex1"), 1)

        val bgVertex = """
            attribute vec2 aPos;
            uniform vec2 uScale;
            varying vec2 vPx;
            void main() {
              gl_Position = vec4(aPos.x * uScale.x - 1.0, aPos.y * uScale.y + 1.0, 0.0, 1.0);
              vPx = aPos;
            }
        """.trimIndent()
        val bgFragment = """
            #ifdef GL_FRAGMENT_PRECISION_HIGH
            precision highp float;
            #else
            precision mediump float;
            #endif
            uniform vec4 uRect;
            uniform float uRadius;
            uniform vec4 uColor;
            varying vec2 vPx;
            void main() {
              vec2 centre = (uRect.xy + uRect.zw) * 0.5;
              vec2 half_ = (uRect.zw - uRect.xy) * 0.5;
              vec2 p = abs(vPx - centre) - (half_ - vec2(uRadius));
              float d = length(max(p, 0.0)) + min(max(p.x, p.y), 0.0) - uRadius;
              gl_FragColor = uColor * clamp(0.5 - d, 0.0, 1.0);
            }
        """.trimIndent()
        bgProgram = link(bgVertex, bgFragment)
        bgPos = GLES20.glGetAttribLocation(bgProgram, "aPos")
        bgScale = GLES20.glGetUniformLocation(bgProgram, "uScale")
        bgRect = GLES20.glGetUniformLocation(bgProgram, "uRect")
        bgRadius = GLES20.glGetUniformLocation(bgProgram, "uRadius")
        bgColor = GLES20.glGetUniformLocation(bgProgram, "uColor")
        resourcesReady = true
    }

    private fun compile(type: Int, source: String): Int {
        val shader = GLES20.glCreateShader(type)
        if (shader == 0) throw GlInitException("glCreateShader failed")
        GLES20.glShaderSource(shader, source)
        GLES20.glCompileShader(shader)
        val status = IntArray(1)
        GLES20.glGetShaderiv(shader, GLES20.GL_COMPILE_STATUS, status, 0)
        if (status[0] == 0) {
            val log = GLES20.glGetShaderInfoLog(shader)
            GLES20.glDeleteShader(shader)
            Log.e(TAG, "lyrics gl: shader compile failed: $log")
            throw GlInitException("shader compile failed: $log")
        }
        return shader
    }

    private fun link(vertexSource: String, fragmentSource: String): Int {
        val vertex = compile(GLES20.GL_VERTEX_SHADER, vertexSource)
        val fragment = compile(GLES20.GL_FRAGMENT_SHADER, fragmentSource)
        val program = GLES20.glCreateProgram()
        GLES20.glAttachShader(program, vertex)
        GLES20.glAttachShader(program, fragment)
        GLES20.glLinkProgram(program)
        GLES20.glDeleteShader(vertex)
        GLES20.glDeleteShader(fragment)
        val status = IntArray(1)
        GLES20.glGetProgramiv(program, GLES20.GL_LINK_STATUS, status, 0)
        if (status[0] == 0) {
            val log = GLES20.glGetProgramInfoLog(program)
            GLES20.glDeleteProgram(program)
            Log.e(TAG, "lyrics gl: program link failed: $log")
            throw GlInitException("program link failed: $log")
        }
        return program
    }

    private fun checkError(label: String) {
        if (!LyricsTrace.on) return
        val error = GLES20.glGetError()
        if (error != GLES20.GL_NO_ERROR) Log.w(TAG, "lyrics gl: $label error 0x${Integer.toHexString(error)}")
    }

    // ---- drawing

    private fun quad(x0: Float, y0: Float, x1: Float, y1: Float, u0: Float, v0: Float, u1: Float, v1: Float) {
        vertices.clear()
        vertices.put(x0).put(y0).put(u0).put(v0)
        vertices.put(x1).put(y0).put(u1).put(v0)
        vertices.put(x0).put(y1).put(u0).put(v1)
        vertices.put(x1).put(y1).put(u1).put(v1)
    }

    private fun drawQuad(posLocation: Int, uvLocation: Int) {
        vertices.position(0)
        GLES20.glVertexAttribPointer(posLocation, 2, GLES20.GL_FLOAT, false, 16, vertices)
        GLES20.glEnableVertexAttribArray(posLocation)
        if (uvLocation >= 0) {
            vertices.position(2)
            GLES20.glVertexAttribPointer(uvLocation, 2, GLES20.GL_FLOAT, false, 16, vertices)
            GLES20.glEnableVertexAttribArray(uvLocation)
        }
        GLES20.glDrawArrays(GLES20.GL_TRIANGLE_STRIP, 0, 4)
    }

    private var missing = false

    /**
     * Draws `p` into the current surface of `width` x `height` px. Returns false when some wanted texture was not ready
     * (it is skipped and requested; the caller draws again when it has landed).
     */
    fun draw(state: RenderState, p: FrameParams, width: Int, height: Int): Boolean {
        val look = state.look
        val layout = look.layout
        missing = false
        GLES20.glViewport(0, 0, width, height)
        GLES20.glClearColor(0f, 0f, 0f, 0f)
        GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT)
        GLES20.glEnable(GLES20.GL_BLEND)
        GLES20.glBlendFunc(GLES20.GL_ONE, GLES20.GL_ONE_MINUS_SRC_ALPHA)
        val scaleX = 2f / width
        val scaleY = -2f / height
        if (p.background) {
            GLES20.glUseProgram(bgProgram)
            GLES20.glUniform2f(bgScale, scaleX, scaleY)
            val left = layout.areaLeft.toFloat()
            val top = layout.areaTop.toFloat()
            val right = left + layout.areaWidthPx
            val bottom = top + layout.areaH
            GLES20.glUniform4f(bgRect, left, top, right, bottom)
            GLES20.glUniform1f(bgRadius, minOf(look.backgroundRadiusPx, (right - left) / 2f, (bottom - top) / 2f))
            val argb = look.backgroundArgb
            val a = (argb ushr 24 and 0xff) / 255f
            // Premultiplied.
            GLES20.glUniform4f(bgColor, (argb shr 16 and 0xff) / 255f * a, (argb shr 8 and 0xff) / 255f * a, (argb and 0xff) / 255f * a, a)
            quad(left - 1f, top - 1f, right + 1f, bottom + 1f, 0f, 0f, 0f, 0f)
            drawQuad(bgPos, -1)
        }
        if (p.visible) {
            GLES20.glUseProgram(textProgram)
            GLES20.glUniform2f(textScale, scaleX, scaleY)
            GLES20.glUniform1f(textAreaLeft, layout.areaLeft.toFloat())
            GLES20.glUniform1fv(textStopX, LyricsMasks.STOPS, if (p.contMask) look.contMaskX else look.noneMaskX, 0)
            GLES20.glUniform1fv(textStopV, LyricsMasks.STOPS, if (p.contMask) look.contMaskV else look.noneMaskV, 0)
            if (p.strip) drawStrip(state, p) else drawLines(state, p)
        }
        checkError("draw")
        return !missing
    }

    private fun drawStrip(state: RenderState, p: FrameParams) {
        val strip = state.model.strip ?: return
        val layout = state.look.layout
        val areaW = layout.areaWidthPx.toFloat()
        val last = minOf(p.lastCell, strip.texts.size - 1)
        for (index in maxOf(0, p.firstCell)..last) {
            val local = strip.cells[index] + p.offset
            if (local > areaW || local + strip.widths[index] < 0f) continue
            val x = layout.areaLeft + local
            drawLine(layout, strip.mainKeys[index], null, x, -1f)
            if (layout.reserveTranslation) drawLine(layout, strip.secondKeys[index], null, x, -1f)
        }
    }

    private fun drawLines(state: RenderState, p: FrameParams) {
        val model = state.model
        val layout = state.look.layout
        if (p.mainLine != FrameParams.NO_LINE) {
            val key = if (p.mainLine == FrameParams.NOTE_LINE) model.noteKey else model.lineKeys[p.mainLine]
            val accent = if (!p.karaokeX.isNaN() && p.mainLine >= 0) model.accentKeys[p.mainLine] else null
            drawLine(layout, key, accent, layout.areaLeft + p.mainX, p.karaokeX)
        }
        if (p.secondLine != FrameParams.NO_LINE && layout.reserveTranslation) {
            drawLine(layout, model.translationKeys[p.secondLine], null, layout.areaLeft + p.secondX, -1f)
        }
    }

    /**
     * One line image at `x` (the text's left edge in surface px). With `accentKey` and a karaoke edge, the accent copy
     * shows left of `edge` (area-local px) and the text-colour copy right of it.
     */
    private fun drawLine(layout: LyricsLayout, key: TexKey, accentKey: TexKey?, x: Float, edge: Float) {
        if (key.text.isEmpty()) return
        val base = store.get(key)
        if (base == null) {
            store.request(key)
            missing = true
            return
        }
        if (base.textures.isEmpty()) return
        var accent: GlLine? = null
        if (accentKey != null) {
            accent = store.get(accentKey)
            if (accent == null) {
                // The accent copy is still being made: the whole line in the text colour until it arrives.
                store.request(accentKey)
                missing = true
            } else if (accent.textures.size != base.textures.size) {
                accent = null
            }
        }
        val row = if (key.row == 0) layout.mainRowTop else layout.secondRowTop
        val rowH = if (key.row == 0) layout.mainRowH else layout.secondRowH
        val ascent = if (key.row == 0) layout.mainAscent else layout.secondAscent
        val descent = if (key.row == 0) layout.mainDescent else layout.secondDescent
        // Vertical position is a whole pixel; horizontal may be fractional.
        val top = Math.round(layout.imageTop(row, rowH, ascent, descent, base.pad)).toFloat()
        val bottom = top + base.imageH
        val areaRight = layout.areaLeft + layout.areaWidthPx.toFloat()
        val karaoke = accent != null && !edge.isNaN()
        GLES20.glUniform1f(textKaraoke, if (karaoke) 1f else 0f)
        if (karaoke) GLES20.glUniform1f(textEdge, edge)
        for (k in base.tiles.indices) {
            val tile = base.tiles[k]
            val left = x - base.pad + tile.srcX
            val right = left + tile.width
            if (right < layout.areaLeft || left > areaRight) continue
            if (karaoke) {
                GLES20.glActiveTexture(GLES20.GL_TEXTURE1)
                GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, accent!!.textures[k])
                GLES20.glActiveTexture(GLES20.GL_TEXTURE0)
            }
            GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, base.textures[k])
            quad(left, top, right, bottom, TilePlan.u0(tile.width), 0f, TilePlan.u1(tile.width), 1f)
            drawQuad(textPos, textUv)
        }
    }

    // ---- prefetch

    private var prefetchModel: RenderModel? = null
    private var prefetchGeneration = -1
    private var prefetchStrip = false
    private var prefetchA = Int.MIN_VALUE
    private var prefetchB = Int.MIN_VALUE
    private var prefetchLocked = false
    private var prefetchWidth = -1

    /**
     * Asks for the textures around what is shown and drops the ones beyond (the wanted set), only when the window, the
     * lyrics, the look or the width changed: the strip's cells within two area widths each side of the view nearest the
     * centre first; in the single-line look the shown line, the next, the previous and the one after the next with
     * their karaoke copies and translations. Never called per frame for an unchanged window.
     */
    fun prefetch(state: RenderState, p: FrameParams) {
        val model = state.model
        val look = state.look
        val layout = look.layout
        val a: Int
        val b: Int
        if (p.strip) {
            val strip = model.strip ?: return
            val window = FloatingLyricsRules.stripWindowPacked(strip.cells, strip.widths, p.offset, layout.areaWidthPx.toFloat(), 2f * layout.areaWidthPx)
            a = FloatingLyricsRules.windowFirst(window)
            b = FloatingLyricsRules.windowLast(window)
        } else {
            a = p.lineIndex
            b = model.linesVersion
        }
        if (prefetchModel === model && prefetchGeneration == look.generation && prefetchStrip == p.strip && prefetchA == a &&
            prefetchB == b && prefetchLocked == look.locked && prefetchWidth == layout.areaWidthPx
        ) {
            return
        }
        prefetchModel = model
        prefetchGeneration = look.generation
        prefetchStrip = p.strip
        prefetchA = a
        prefetchB = b
        prefetchLocked = look.locked
        prefetchWidth = layout.areaWidthPx
        val wanted = LinkedHashSet<TexKey>()
        if (p.strip) {
            val strip = model.strip ?: return
            if (a <= b) {
                val visible = FloatingLyricsRules.stripWindowPacked(strip.cells, strip.widths, p.offset, layout.areaWidthPx.toFloat(), 0f)
                val centre = if (FloatingLyricsRules.windowFirst(visible) <= FloatingLyricsRules.windowLast(visible)) {
                    (FloatingLyricsRules.windowFirst(visible) + FloatingLyricsRules.windowLast(visible)) / 2
                } else {
                    a
                }.coerceIn(a, b)
                val reach = maxOf(centre - a, b - centre)
                for (distance in 0..reach) {
                    addStripCell(wanted, strip, centre - distance, a, b, layout)
                    if (distance > 0) addStripCell(wanted, strip, centre + distance, a, b, layout)
                }
            }
        } else {
            val index = p.lineIndex
            val order = if (index >= 0) intArrayOf(index, index + 1, index - 1, index + 2) else intArrayOf(0, 1)
            for (at in order) {
                if (at < 0 || at >= model.texts.size) continue
                if (model.texts[at].isNotEmpty()) {
                    wanted.add(TexKey(0, model.texts[at], false))
                    if (model.karaoke[at] != null) wanted.add(TexKey(0, model.texts[at], true))
                }
                if (layout.reserveTranslation && model.translations[at].isNotEmpty()) wanted.add(TexKey(1, model.translations[at], false))
            }
            if (!look.locked) wanted.add(TexKey(0, RenderModel.NOTE, false))
        }
        store.setLooks(look, true)
        store.retain(wanted)
        for (key in wanted) store.request(key)
    }

    private fun addStripCell(wanted: MutableSet<TexKey>, strip: StripModel, index: Int, first: Int, last: Int, layout: LyricsLayout) {
        if (index < first || index > last || index < 0 || index >= strip.texts.size) return
        if (strip.texts[index].isNotEmpty()) wanted.add(TexKey(0, strip.texts[index], false))
        if (layout.reserveTranslation && strip.translations[index].isNotEmpty()) wanted.add(TexKey(1, strip.translations[index], false))
    }

    /** Forces the next [prefetch] to run (after the textures were dropped). */
    fun invalidatePrefetch() {
        prefetchModel = null
    }
}
