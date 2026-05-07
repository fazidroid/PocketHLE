package com.pockethle.app

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Rect
import android.os.Bundle
import android.util.Base64
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.widget.ProgressBar
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.appcompat.widget.Toolbar
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

/**
 * Hosts the emulator output for one game. The current implementation
 * runs the emulator to completion on a background coroutine and then
 * paints the captured framebuffer into a [SurfaceView] (a real-time
 * loop will land in a follow-up PR once the renderer settles).
 */
class GameActivity : AppCompatActivity(), SurfaceHolder.Callback {

    private lateinit var surface: SurfaceView
    private lateinit var progress: ProgressBar
    private lateinit var status: TextView
    private var pendingFrame: FrameSnapshot? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_game)
        setSupportActionBar(findViewById<Toolbar>(R.id.toolbar))
        supportActionBar?.setDisplayHomeAsUpEnabled(true)

        val name = intent.getStringExtra(EXTRA_GAME_NAME) ?: "PocketHLE"
        title = name

        surface = findViewById(R.id.surface)
        surface.holder.addCallback(this)
        progress = findViewById(R.id.progress)
        status = findViewById(R.id.status)

        val id = intent.getStringExtra(EXTRA_GAME_ID)
        if (id == null) {
            status.text = getString(R.string.run_failed_no_id)
            progress.visibility = android.view.View.GONE
            return
        }

        val rootDir = LibraryPaths.root(this)
        lifecycleScope.launch {
            val raw = withContext(Dispatchers.IO) {
                NativeBridge.runGame(rootDir, id)
            }
            progress.visibility = android.view.View.GONE
            applyOutcome(raw)
        }
    }

    override fun onSupportNavigateUp(): Boolean {
        finish()
        return true
    }

    private fun applyOutcome(raw: String) {
        try {
            val obj = JSONObject(raw)
            val ok = obj.optBoolean("ok", true)
            val summary = obj.optString("summary", "(no summary)")
            val err = obj.optString("error", "")
            status.text = if (ok) summary else "$summary\n$err"
            val frameObj = obj.optJSONObject("frame") ?: return
            val frame = FrameSnapshot(
                width = frameObj.getInt("width"),
                height = frameObj.getInt("height"),
                rgba = Base64.decode(frameObj.getString("rgba_b64"), Base64.DEFAULT),
            )
            pendingFrame = frame
            paintFrame(frame)
        } catch (e: Exception) {
            status.text = "Could not parse run result: ${e.message}\n$raw"
        }
    }

    private fun paintFrame(frame: FrameSnapshot) {
        val holder = surface.holder
        val canvas = holder.lockCanvas() ?: return
        try {
            canvas.drawColor(Color.BLACK)
            val bitmap = Bitmap.createBitmap(frame.width, frame.height, Bitmap.Config.ARGB_8888)
            val pixelInts = IntArray(frame.width * frame.height)
            var i = 0
            var p = 0
            while (i + 3 < frame.rgba.size) {
                val r = frame.rgba[i].toInt() and 0xff
                val g = frame.rgba[i + 1].toInt() and 0xff
                val b = frame.rgba[i + 2].toInt() and 0xff
                val a = frame.rgba[i + 3].toInt() and 0xff
                pixelInts[p++] = (a shl 24) or (r shl 16) or (g shl 8) or b
                i += 4
            }
            bitmap.setPixels(pixelInts, 0, frame.width, 0, 0, frame.width, frame.height)
            val w = canvas.width
            val h = canvas.height
            val scale = minOf(
                w.toFloat() / frame.width,
                h.toFloat() / frame.height,
            )
            val dstW = (frame.width * scale).toInt()
            val dstH = (frame.height * scale).toInt()
            val left = (w - dstW) / 2
            val top = (h - dstH) / 2
            val src = Rect(0, 0, frame.width, frame.height)
            val dst = Rect(left, top, left + dstW, top + dstH)
            canvas.drawBitmap(bitmap, src, dst, Paint(Paint.FILTER_BITMAP_FLAG))
        } finally {
            holder.unlockCanvasAndPost(canvas)
        }
    }

    // SurfaceHolder.Callback ------------------------------------------------

    override fun surfaceCreated(holder: SurfaceHolder) {
        pendingFrame?.let { paintFrame(it) }
    }

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
        pendingFrame?.let { paintFrame(it) }
    }

    override fun surfaceDestroyed(holder: SurfaceHolder) = Unit

    private data class FrameSnapshot(
        val width: Int,
        val height: Int,
        val rgba: ByteArray,
    )

    companion object {
        const val EXTRA_GAME_ID = "com.pockethle.app.EXTRA_GAME_ID"
        const val EXTRA_GAME_NAME = "com.pockethle.app.EXTRA_GAME_NAME"

        @Suppress("unused")
        private fun ignore(@Suppress("UNUSED_PARAMETER") b: BitmapFactory) = Unit
    }
}
