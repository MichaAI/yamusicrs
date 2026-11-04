package xyz.neodustry.yamusicrs

import android.app.Application
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.LruCache
import java.io.File
import java.net.URL
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.yamusic_android.Core
import uniffi.yamusic_android.CoreListener
import uniffi.yamusic_android.LyricsData
import uniffi.yamusic_android.PlayerState
import uniffi.yamusic_android.TrackItem
import uniffi.yamusic_android.WavePreset

data class Toast(val text: String, val error: Boolean)

/** Текст играющего трека: `lyrics == null` — текста нет. */
data class TrackLyrics(val trackId: String, val lyrics: LyricsData?)

/** Ядро на весь процесс: его делят экран и медиасервис. Колбэки приходят из потока движка. */
class YaApp : Application(), CoreListener {
    val state = MutableStateFlow<PlayerState?>(null)
    val likes = MutableStateFlow<List<TrackItem>>(emptyList())
    val queue = MutableStateFlow<Pair<String, List<TrackItem>>>("" to emptyList())
    val wheel = MutableStateFlow<List<WavePreset>>(emptyList())
    val lyrics = MutableStateFlow<TrackLyrics?>(null)
    /** Палитра под обложку играющего трека, как тема TUI. */
    val palette = MutableStateFlow(Pal.Default)
    val toasts = MutableSharedFlow<Toast>(extraBufferCapacity = 16)

    lateinit var core: Core
        private set

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

    /** Обложки и миниатюры в памяти: до 48 МБ. */
    private val covers = object : LruCache<String, Bitmap>(48 * 1024 * 1024) {
        override fun sizeOf(key: String, value: Bitmap) = value.allocationByteCount
    }

    override fun onCreate() {
        super.onCreate()
        val music = getExternalFilesDir("music") ?: File(filesDir, "music")
        core = Core(filesDir.absolutePath, cacheDir.absolutePath, music.absolutePath, this)
        scope.launch {
            state.map { it?.track?.cover }.distinctUntilChanged().collectLatest { url ->
                val bmp = url?.let { cover(it) } ?: return@collectLatest
                palette.value = withContext(Dispatchers.Default) { Pal.from(bmp) }
            }
        }
    }

    override fun onState(state: PlayerState) {
        this.state.value = state
    }

    override fun onLikes(likes: List<TrackItem>) {
        this.likes.value = likes
    }

    override fun onQueue(title: String, items: List<TrackItem>) {
        queue.value = title to items
    }

    override fun onToast(text: String, error: Boolean) {
        toasts.tryEmit(Toast(text, error))
    }

    override fun onWheel(presets: List<WavePreset>) {
        wheel.value = presets
    }

    override fun onLyrics(trackId: String, lyrics: LyricsData?) {
        this.lyrics.value = TrackLyrics(trackId, lyrics)
    }

    /** Картинка по https: из памяти или сети. */
    suspend fun cover(url: String): Bitmap? {
        covers.get(url)?.let { return it }
        val bmp = withContext(Dispatchers.IO) {
            runCatching { URL(url).openStream().use(BitmapFactory::decodeStream) }.getOrNull()
        } ?: return null
        covers.put(url, bmp)
        return bmp
    }

    fun cached(url: String?): Bitmap? = url?.let { covers.get(it) }
}
