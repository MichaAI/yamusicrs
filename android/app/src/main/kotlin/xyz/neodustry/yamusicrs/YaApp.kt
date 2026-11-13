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
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
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

/** Сообщение поверх экрана: ошибка или то, что иначе не видно (экспорт); `action` — кнопка. */
data class UiToast(val text: String, val error: Boolean, val action: ToastAction? = null, val id: Long = System.nanoTime())

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

    /** Что показать поверх экрана (см. [UiToast]). */
    val toast = MutableStateFlow<UiToast?>(null)

    /** Идёт обновление по жесту «потянуть вниз». */
    var refreshing by androidx.compose.runtime.mutableStateOf(false)
        private set

    /** Страница любимых исполнителей (вкладка «Коллекция»). */
    var artistsId by androidx.compose.runtime.mutableStateOf<ULong?>(null)

    /**
     * Потянули вниз: лайки, плейлисты, подборки, пресеты волны, новинки и исполнители перечитываются
     * с сервера. Старое остаётся на экране, пока не придёт новое.
     */
    fun refresh() {
        if (refreshing) return
        refreshing = true
        core.refresh()
        val (oldReleases, oldArtists) = releasesId to artistsId
        if (oldReleases != null) releasesId = core.releases()
        if (oldArtists != null) artistsId = core.likedArtists()
        scope.launch {
            // Индикатор держится, пока не пришли новые страницы (но не меньше 0,6 с и не больше 6 с).
            val t0 = System.currentTimeMillis()
            kotlinx.coroutines.withTimeoutOrNull(6000) {
                pages.first { p -> (releasesId == oldReleases || releasesId in p) && (artistsId == oldArtists || artistsId in p) }
            }
            kotlinx.coroutines.delay((600 - (System.currentTimeMillis() - t0)).coerceAtLeast(0))
            // Прежние страницы закрываем, когда новые уже на экране: список не мигает пустым.
            if (oldReleases != null && oldReleases != releasesId) closePage(oldReleases)
            if (oldArtists != null && oldArtists != artistsId) closePage(oldArtists)
            refreshing = false
        }
    }

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
        // Палитра прошлого запуска — сразу: тема не перекрашивается на глазах, пока грузится обложка.
        loadPalette()?.let { palette.value = it }
        core = Core(filesDir.absolutePath, cacheDir.absolutePath, music.absolutePath, name, this)
        watchNetwork()
        scope.launch {
            state.map { it?.track?.cover }.distinctUntilChanged().collectLatest { url ->
                val bmp = url?.let { cover(it) } ?: return@collectLatest
                val pal = withContext(Dispatchers.Default) { Pal.from(bmp) }
                palette.value = pal
                savePalette(pal)
            }
        }
        scope.launch(Dispatchers.IO) {
            kotlinx.coroutines.delay(8000)
            trimCovers()
        }
    }

    private fun prefs() = getSharedPreferences("ui", MODE_PRIVATE)

    private fun savePalette(p: Pal) {
        val ints = (listOf(p.accent, p.accent2, p.surface) + p.blob).joinToString(",") { it.value.toString() }
        prefs().edit().putString("palette", ints).apply()
    }

    private fun loadPalette(): Pal? = runCatching {
        val v = prefs().getString("palette", null)?.split(',')?.map { androidx.compose.ui.graphics.Color(it.toULong()) } ?: return null
        if (v.size < 4) null else Pal(v[0], v[1], v[2], v.drop(3))
    }.getOrNull()

    override fun onState(state: PlayerState) {
        this.state.value = state
    }

    override fun onLikes(likes: List<TrackItem>) {
        this.likes.value = likes
    }

    override fun onQueue(title: String, items: List<TrackItem>) {
        queue.value = title to items
    }

    /**
     * Обычные сообщения ядра («качество такое-то», «в очередь») на телефоне и так видны на экране —
     * они идут в лог. Показываем то, чего иначе не узнать: ошибки и сообщения с кнопкой.
     */
    override fun onToast(text: String, error: Boolean, action: ToastAction?) {
        if (error) android.util.Log.w("yamusicrs", text) else android.util.Log.i("yamusicrs", text)
        // Значки Nerd Font из сообщений TUI в шрифте телефона — пустые квадраты.
        val clean = text.filterNot { it in '\uE000'..'\uF8FF' }.trim()
        // «Продолжаем с того же места» при каждом запуске — шум: трек и так стоит в мини-плеере.
        if (action == ToastAction.RESUME && !error) return
        if (clean.isNotEmpty()) toast.value = UiToast(clean, error, action)
    }

    fun say(text: String, error: Boolean = false) {
        toast.value = UiToast(text, error)
    }

    /**
     * Экспорт готов: ядро положило файл во временную папку, переносим в общую «Музыку»
     * (Music/yamusicrs) — оттуда его видят плееры, файловый менеджер и компьютер по USB.
     */
    override fun onExported(path: String, name: String) {
        scope.launch(Dispatchers.IO) {
            val src = File(path)
            val where = runCatching { publish(src, name) }
            src.delete()
            where.fold(
                { say("Сохранено: $it") },
                { e -> ylog("экспорт не перенёсся: $e"); say("Не удалось сохранить «$name»: ${e.message}", error = true) },
            )
        }
    }

    private fun publish(src: File, name: String): String {
        if (android.os.Build.VERSION.SDK_INT < 29) {
            // До Android 10 в общую «Музыку» без разрешения не попасть — кладём в свою папку на накопителе.
            val dir = File(getExternalFilesDir(null), "export").apply { mkdirs() }
            src.copyTo(File(dir, name), overwrite = true)
            return "Android/data/$packageName/files/export/$name"
        }
        val mime = when (name.substringAfterLast('.').lowercase()) {
            "flac" -> "audio/flac"
            "mp3" -> "audio/mpeg"
            "m4a" -> "audio/mp4"
            "wav" -> "audio/x-wav"
            else -> "audio/aac"
        }
        val values = android.content.ContentValues().apply {
            put(android.provider.MediaStore.Audio.Media.DISPLAY_NAME, name)
            put(android.provider.MediaStore.Audio.Media.MIME_TYPE, mime)
            put(android.provider.MediaStore.Audio.Media.RELATIVE_PATH, "Music/yamusicrs")
            put(android.provider.MediaStore.Audio.Media.IS_PENDING, 1)
        }
        val col = android.provider.MediaStore.Audio.Media.getContentUri(android.provider.MediaStore.VOLUME_EXTERNAL_PRIMARY)
        val uri = contentResolver.insert(col, values) ?: error("хранилище не приняло файл")
        try {
            contentResolver.openOutputStream(uri).use { out -> src.inputStream().use { it.copyTo(out ?: error("нет доступа на запись")) } }
            contentResolver.update(uri, android.content.ContentValues().apply { put(android.provider.MediaStore.Audio.Media.IS_PENDING, 0) }, null, null)
        } catch (e: Exception) {
            contentResolver.delete(uri, null, null)
            throw e
        }
        return "Музыка/yamusicrs/$name"
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

    private val coverDir by lazy { File(cacheDir, "covers").apply { mkdirs() } }

    /** Идущие загрузки: десять строк с одной обложкой — один запрос. */
    private val loading = java.util.concurrent.ConcurrentHashMap<String, kotlinx.coroutines.Deferred<Bitmap?>>()

    private fun coverFile(url: String): File {
        val h = java.security.MessageDigest.getInstance("SHA-1").digest(url.toByteArray())
        return File(coverDir, h.joinToString("") { "%02x".format(it) })
    }

    /**
     * Картинка по https: из памяти, с диска или из сети. На диске (cache/covers) она переживает
     * перезапуск — при старте обложки и палитра есть сразу, и без сети тоже.
     */
    suspend fun cover(url: String): Bitmap? {
        covers.get(url)?.let { return it }
        // Загрузка живёт в области приложения: строку пролистали — картинка всё равно доедет в кеш.
        val job = loading.getOrPut(url) {
            scope.async(Dispatchers.IO) {
                val f = coverFile(url)
                val bmp = f.takeIf { it.exists() }?.let { BitmapFactory.decodeFile(it.path) }?.also { f.setLastModified(System.currentTimeMillis()) }
                    ?: runCatching {
                        val bytes = URL(url).openConnection().apply { connectTimeout = 10_000; readTimeout = 15_000 }
                            .getInputStream().use { it.readBytes() }
                        BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.also {
                            val tmp = File(coverDir, f.name + ".part")
                            tmp.writeBytes(bytes)
                            tmp.renameTo(f)
                        }
                    }.getOrNull()
                if (bmp != null) covers.put(url, bmp)
                bmp
            }.also { j -> j.invokeOnCompletion { loading.remove(url, j) } }
        }
        return job.await()
    }

    /** Кеш обложек на диске — не больше 200 МБ: давно не открывавшиеся удаляются. */
    private fun trimCovers() {
        val files = coverDir.listFiles()?.sortedBy { it.lastModified() } ?: return
        var total = files.sumOf { it.length() }
        for (f in files) {
            if (total <= 200L * 1024 * 1024) break
            total -= f.length()
            f.delete()
        }
    }

    fun cached(url: String?): Bitmap? = url?.let { covers.get(it) }
}
