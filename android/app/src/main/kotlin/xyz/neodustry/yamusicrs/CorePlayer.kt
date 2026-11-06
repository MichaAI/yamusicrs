package xyz.neodustry.yamusicrs

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.net.Uri
import android.os.Looper
import android.os.PowerManager
import androidx.core.content.ContextCompat
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.Player
import androidx.media3.common.SimpleBasePlayer
import com.google.common.util.concurrent.Futures
import com.google.common.util.concurrent.ListenableFuture
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import uniffi.yamusic_android.PlayerState

/**
 * Фасад Media3 над ядром: звук выводит Rust (AAudio), а этот «плеер» только показывает
 * состояние ядра сессии (уведомление, экран блокировки, Bluetooth) и передаёт ему команды.
 * Заодно держит аудиофокус, паузу при отключении наушников и wakelock на время игры.
 */
class CorePlayer(private val app: YaApp) : SimpleBasePlayer(Looper.getMainLooper()) {
    private val scope = MainScope()
    private var cur: PlayerState? = null

    private val audio = app.getSystemService(AudioManager::class.java)
    private val focusRequest = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
        .setAudioAttributes(
            AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                .build(),
        )
        .setOnAudioFocusChangeListener(::onFocus)
        .build()
    private var hasFocus = false
    /** Фокус временно забрали (звонок, навигатор) — когда вернут, играть дальше. */
    private var resumeOnGain = false

    private val wake = app.getSystemService(PowerManager::class.java)
        .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "yamusicrs:playback")
        .apply { setReferenceCounted(false) }

    /** Wi-Fi не засыпает, пока играет: трек докачивается потоком, обрыв — тишина. */
    private val wifi = app.getSystemService(android.net.wifi.WifiManager::class.java)
        .createWifiLock(android.net.wifi.WifiManager.WIFI_MODE_FULL_HIGH_PERF, "yamusicrs:stream")
        .apply { setReferenceCounted(false) }

    private val noisy = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action == AudioManager.ACTION_AUDIO_BECOMING_NOISY) {
                ylog("наушники отключились — пауза")
                app.core.pause()
            }
        }
    }
    private var noisyRegistered = false

    init {
        scope.launch {
            app.state.collect { s ->
                val was = cur.playing()
                cur = s
                if (s.playing() != was) onPlayingChanged(s.playing())
                invalidateState()
            }
        }
        // Соседи в плейлисте сессии берутся из очереди.
        scope.launch { app.queue.collect { invalidateState() } }
    }

    private fun PlayerState?.playing() = this != null && track != null && !paused

    private fun onPlayingChanged(playing: Boolean) {
        if (playing) {
            if (!hasFocus) {
                hasFocus = audio.requestAudioFocus(focusRequest) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
            }
            if (!noisyRegistered) {
                ContextCompat.registerReceiver(
                    app,
                    noisy,
                    IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY),
                    ContextCompat.RECEIVER_NOT_EXPORTED,
                )
                noisyRegistered = true
            }
            wake.acquire(6 * 60 * 60 * 1000L)
            if (!wifi.isHeld) wifi.acquire()
        } else {
            if (noisyRegistered) {
                app.unregisterReceiver(noisy)
                noisyRegistered = false
            }
            if (wake.isHeld) wake.release()
            if (wifi.isHeld) wifi.release()
        }
    }

    private fun onFocus(change: Int) {
        ylog("аудиофокус: $change")
        when (change) {
            AudioManager.AUDIOFOCUS_LOSS -> {
                hasFocus = false
                resumeOnGain = false
                app.core.pause()
            }
            AudioManager.AUDIOFOCUS_LOSS_TRANSIENT, AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK -> {
                resumeOnGain = cur.playing()
                app.core.pause()
            }
            AudioManager.AUDIOFOCUS_GAIN -> {
                hasFocus = true
                if (resumeOnGain) app.core.play()
                resumeOnGain = false
            }
        }
    }

    override fun getState(): State {
        val s = cur
        val commands = Player.Commands.Builder()
            .addAll(
                COMMAND_PLAY_PAUSE,
                COMMAND_STOP,
                COMMAND_SEEK_TO_NEXT,
                COMMAND_SEEK_TO_NEXT_MEDIA_ITEM,
                COMMAND_SEEK_TO_PREVIOUS,
                COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM,
                COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM,
                COMMAND_SEEK_TO_MEDIA_ITEM,
                COMMAND_GET_CURRENT_MEDIA_ITEM,
                COMMAND_GET_TIMELINE,
                COMMAND_GET_METADATA,
                // Без него SimpleBasePlayer.release() молча ничего не делает: при пересоздании
                // сервиса старый плеер оставался жив, держал фокус и ставил паузу новому.
                COMMAND_RELEASE,
            )
            .build()
        val b = State.Builder().setAvailableCommands(commands)
        val t = s?.track ?: return b.setPlaybackState(STATE_IDLE).build()
        val meta = MediaMetadata.Builder()
            .setTitle(t.title)
            .setArtist(t.artists)
            .setAlbumTitle(t.album)
            .setArtworkUri(t.cover?.let(Uri::parse))
            .build()
        // Плейлист сессии — предыдущий, текущий и следующий трек. С одним треком Media3 решает,
        // что дальше ничего нет, и молча глотает «следующий» от внешних пультов (KDE Connect,
        // часы, Android Auto): BasePlayer.seekToNext() без следующего элемента не доходит до handleSeek.
        val items = app.queue.value.second
        val pos = s.queuePos?.toInt()
        val inQueue = pos != null && pos in items.indices && items[pos].id == t.id
        val hasPrev = pos != null && pos > 0
        val hasNext = s.wave || (pos != null && pos + 1 < s.queueLen.toInt()) || app.settings.value?.waveAfterList == true
        fun neighbour(uid: String, n: uniffi.yamusic_android.TrackItem?): MediaItemData {
            val m = MediaMetadata.Builder()
                .setTitle(n?.title ?: "")
                .setArtist(n?.artists)
                .setArtworkUri(n?.cover?.let(Uri::parse))
                .build()
            return MediaItemData.Builder(uid)
                .setMediaItem(MediaItem.Builder().setMediaId(uid).setMediaMetadata(m).build())
                .build()
        }
        val item = MediaItem.Builder().setMediaId(t.id).setMediaMetadata(meta).build()
        val data = MediaItemData.Builder("cur:${t.id}")
            .setMediaItem(item)
            .setDurationUs(t.durationMs.toLong() * 1000)
            .setIsSeekable(true)
            .build()
        val list = buildList {
            if (hasPrev) add(neighbour("prev", if (inQueue) items.getOrNull(pos!! - 1) else null))
            add(data)
            if (hasNext) add(neighbour("next", if (inQueue) items.getOrNull(pos!! + 1) else null))
        }
        curIndex = if (hasPrev) 1 else 0
        val playing = !s.paused
        return b.setPlaylist(list)
            .setCurrentMediaItemIndex(curIndex)
            .setPlaybackState(if (s.loading) STATE_BUFFERING else STATE_READY)
            .setPlayWhenReady(playing, PLAY_WHEN_READY_CHANGE_REASON_USER_REQUEST)
            .setContentPositionMs(
                PositionSupplier.getExtrapolating(s.positionMs.toLong(), if (playing && !s.loading) 1f else 0f),
            )
            .build()
    }

    override fun handleSetPlayWhenReady(playWhenReady: Boolean): ListenableFuture<*> {
        ylog("медиасессия: playWhenReady=$playWhenReady")
        if (playWhenReady) app.core.play() else app.core.pause()
        return Futures.immediateVoidFuture()
    }

    /** Где в плейлисте сессии текущий трек (0 или 1 — есть ли перед ним предыдущий). */
    private var curIndex = 0

    override fun handleSeek(mediaItemIndex: Int, positionMs: Long, seekCommand: Int): ListenableFuture<*> {
        ylog("медиасессия: seek cmd=$seekCommand item=$mediaItemIndex pos=$positionMs")
        when {
            seekCommand == COMMAND_SEEK_TO_NEXT || seekCommand == COMMAND_SEEK_TO_NEXT_MEDIA_ITEM -> app.core.next()
            seekCommand == COMMAND_SEEK_TO_PREVIOUS || seekCommand == COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM -> app.core.prev()
            // Выбор соседа по номеру (очередь в пульте).
            mediaItemIndex > curIndex -> app.core.next()
            mediaItemIndex < curIndex -> app.core.prevTrack()
            else -> app.core.seek(positionMs.coerceAtLeast(0).toULong())
        }
        return Futures.immediateVoidFuture()
    }

    override fun handleStop(): ListenableFuture<*> {
        ylog("медиасессия: stop")
        app.core.pause()
        return Futures.immediateVoidFuture()
    }

    override fun handleRelease(): ListenableFuture<*> {
        scope.cancel()
        onPlayingChanged(false)
        audio.abandonAudioFocusRequest(focusRequest)
        return Futures.immediateVoidFuture()
    }
}
