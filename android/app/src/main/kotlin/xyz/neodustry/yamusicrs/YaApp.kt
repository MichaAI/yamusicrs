package xyz.neodustry.yamusicrs

import android.app.Application
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.LruCache
import java.io.File
import java.net.URL
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.withContext
import uniffi.yamusic_android.Core
import uniffi.yamusic_android.CoreListener
import uniffi.yamusic_android.PlayerState
import uniffi.yamusic_android.TrackItem

data class Toast(val text: String, val error: Boolean)

/** Ядро на весь процесс: его делят экран и медиасервис. Колбэки приходят из потока движка. */
class YaApp : Application(), CoreListener {
    val state = MutableStateFlow<PlayerState?>(null)
    val likes = MutableStateFlow<List<TrackItem>>(emptyList())
    val queue = MutableStateFlow<Pair<String, List<TrackItem>>>("" to emptyList())
    val toasts = MutableSharedFlow<Toast>(extraBufferCapacity = 16)

    lateinit var core: Core
        private set

    private val covers = LruCache<String, Bitmap>(48)

    override fun onCreate() {
        super.onCreate()
        val music = getExternalFilesDir("music") ?: File(filesDir, "music")
        core = Core(filesDir.absolutePath, cacheDir.absolutePath, music.absolutePath, this)
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

    /** Обложка по https: из памяти или сети. */
    suspend fun cover(url: String): Bitmap? {
        covers.get(url)?.let { return it }
        val bmp = withContext(Dispatchers.IO) {
            runCatching { URL(url).openStream().use(BitmapFactory::decodeStream) }.getOrNull()
        } ?: return null
        covers.put(url, bmp)
        return bmp
    }
}
