package xyz.neodustry.yamusicrs

import android.app.Application
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.LruCache
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import java.io.File
import java.net.URL
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.yamusic_android.Core
import uniffi.yamusic_android.CoreListener
import uniffi.yamusic_android.LyricsData
import uniffi.yamusic_android.Page
import uniffi.yamusic_android.PlayerState
import uniffi.yamusic_android.PlaylistItem
import uniffi.yamusic_android.SettingsData
import uniffi.yamusic_android.ToastAction
import uniffi.yamusic_android.TrackItem
import uniffi.yamusic_android.WavePreset

/** Текст играющего трека: `lyrics == null` — текста нет. */
data class TrackLyrics(val trackId: String, val lyrics: LyricsData?)

/** Строка в общий лог (logcat + файл …/files/logs/yamusicrs.log, его adb читает всегда). */
fun ylog(text: String) {
    android.util.Log.i("yamusicrs", text)
    YaApp.instance?.takeIf { it.ready }?.core?.log(text)
}

/** Ядро на весь процесс: его делят экран и медиасервис. Колбэки приходят из потока движка. */
class YaApp : Application(), CoreListener {
    val state = MutableStateFlow<PlayerState?>(null)
    val likes = MutableStateFlow<List<TrackItem>>(emptyList())
    val queue = MutableStateFlow<Pair<String, List<TrackItem>>>("" to emptyList())
    val wheel = MutableStateFlow<List<WavePreset>>(emptyList())
    val lyrics = MutableStateFlow<TrackLyrics?>(null)
    /** Ответы на открытые страницы по id; нет ключа — ещё грузится. */
    val pages = MutableStateFlow<Map<ULong, Page>>(emptyMap())
    /** Свои плейлисты и «Настроили для вас». */
    val playlists = MutableStateFlow<Pair<List<PlaylistItem>, List<PlaylistItem>>>(emptyList<PlaylistItem>() to emptyList())
    val settings = MutableStateFlow<SettingsData?>(null)
    /** Исполнитель играющего трека: фото и ближайший концерт (для экрана волны). */
    val artist = MutableStateFlow<uniffi.yamusic_android.ArtistCard?>(null)
    /** Страница «Новые релизы» вкладки «Новинки» (запрашивается один раз). */
    var releasesId by androidx.compose.runtime.mutableStateOf<ULong?>(null)
    /** Палитра под обложку играющего трека, как тема TUI. */
    val palette = MutableStateFlow(Pal.Default)

    /** Фото исполнителей, доспрошенные у ядра (id → адрес); у кого нет — не попадают. */
    val artistPhotos = androidx.compose.runtime.mutableStateMapOf<String, String>()
    private val askedPhotos = HashSet<String>()

    /** Спросить фото тех, кого ещё не спрашивали. */
    fun askPhotos(ids: List<String>) {
        val fresh = synchronized(askedPhotos) { ids.filter { it.isNotEmpty() && askedPhotos.add(it) } }
        if (fresh.isNotEmpty()) core.requestArtistPhotos(fresh)
    }

    /** id скачанных треков (без суффикса альбома). */
    var downloaded by androidx.compose.runtime.mutableStateOf<Set<String>>(emptySet())

    override fun onDownloaded(ids: List<String>) {
        downloaded = ids.toHashSet()
    }

    fun isDownloaded(id: String) = id.substringBefore(':') in downloaded

    override fun onArtistPhoto(id: String, photo: String?) {
        if (photo != null) artistPhotos[id] = photo
    }

    lateinit var core: Core
        private set

    internal val ready get() = ::core.isInitialized

    companion object {
        @Volatile
        var instance: YaApp? = null
            private set
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

    /** Обложки и миниатюры в памяти: четверть кучи (48–192 МБ) — листание не выталкивает соседей. */
    private val covers = object : LruCache<String, Bitmap>(
        maxOf(48L * 1024 * 1024, Runtime.getRuntime().maxMemory() / 4).coerceAtMost(192L * 1024 * 1024).toInt(),
    ) {
        override fun sizeOf(key: String, value: Bitmap) = value.allocationByteCount
    }

    /**
     * Тип сети и оценка канала — ядру для качества «Авто» (их же шлёт мобильный клиент)
     * и для докачки «только по Wi-Fi». Ethernet и прочее сервер видит как Wi-Fi, как у официального.
     */
    private fun watchNetwork() {
        val cm = getSystemService(android.net.ConnectivityManager::class.java) ?: return
        cm.registerDefaultNetworkCallback(object : android.net.ConnectivityManager.NetworkCallback() {
            override fun onCapabilitiesChanged(network: android.net.Network, caps: android.net.NetworkCapabilities) {
                val kind = when {
                    caps.hasTransport(android.net.NetworkCapabilities.TRANSPORT_WIFI) -> "wifi"
                    caps.hasTransport(android.net.NetworkCapabilities.TRANSPORT_CELLULAR) -> "mobile"
                    else -> "other"
                }
                val kbps = caps.linkDownstreamBandwidthKbps.takeIf { it > 0 } ?: -1
                core.setNetwork(kind, kbps)
            }

            override fun onLost(network: android.net.Network) {
                core.setNetwork("none", -1)
            }
        })
    }

    override fun onCreate() {
        super.onCreate()
        instance = this
        val music = getExternalFilesDir("music") ?: File(filesDir, "music")
        val name = listOf(android.os.Build.MANUFACTURER, android.os.Build.MODEL)
            .filter { it.isNotBlank() }.distinct().joinToString(" ")
            .replaceFirstChar { it.uppercase() }
        core = Core(filesDir.absolutePath, cacheDir.absolutePath, music.absolutePath, name, this)
        watchNetwork()
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

    /** Тостов на телефоне нет: что важно, видно на экране; остальное — в logcat. */
    override fun onToast(text: String, error: Boolean, action: ToastAction?) {
        if (error) android.util.Log.w("yamusicrs", text) else android.util.Log.i("yamusicrs", text)
    }

    override fun onWheel(presets: List<WavePreset>) {
        wheel.value = presets
    }

    override fun onLyrics(trackId: String, lyrics: LyricsData?) {
        this.lyrics.value = TrackLyrics(trackId, lyrics)
    }

    override fun onPage(id: ULong, page: Page) {
        pages.update { it + (id to page) }
    }

    override fun onPlaylists(own: List<PlaylistItem>, personal: List<PlaylistItem>) {
        playlists.value = own to personal
    }

    override fun onArtist(card: uniffi.yamusic_android.ArtistCard) {
        artist.value = card
    }

    override fun onSettings(settings: SettingsData) {
        this.settings.value = settings
        Viz.core = core
        Viz.kind = settings.visualizer
    }

    /** Страница больше не нужна: забыть и ответ, и треки в ядре. */
    fun closePage(id: ULong) {
        core.closePage(id)
        pages.update { it - id }
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
