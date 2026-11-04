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

    private val noisy = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action == AudioManager.ACTION_AUDIO_BECOMING_NOISY) app.core.pause()
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
        } else {
            if (noisyRegistered) {
                app.unregisterReceiver(noisy)
                noisyRegistered = false
            }
            if (wake.isHeld) wake.release()
        }
    }

    private fun onFocus(change: Int) {
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
                COMMAND_GET_CURRENT_MEDIA_ITEM,
                COMMAND_GET_TIMELINE,
                COMMAND_GET_METADATA,
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
        val item = MediaItem.Builder().setMediaId(t.id).setMediaMetadata(meta).build()
        val data = MediaItemData.Builder(t.id)
            .setMediaItem(item)
            .setDurationUs(t.durationMs.toLong() * 1000)
            .setIsSeekable(true)
            .build()
        val playing = !s.paused
        return b.setPlaylist(listOf(data))
            .setCurrentMediaItemIndex(0)
            .setPlaybackState(if (s.loading) STATE_BUFFERING else STATE_READY)
            .setPlayWhenReady(playing, PLAY_WHEN_READY_CHANGE_REASON_USER_REQUEST)
            .setContentPositionMs(
                PositionSupplier.getExtrapolating(s.positionMs.toLong(), if (playing && !s.loading) 1f else 0f),
            )
            .build()
    }

    override fun handleSetPlayWhenReady(playWhenReady: Boolean): ListenableFuture<*> {
        if (playWhenReady) app.core.play() else app.core.pause()
        return Futures.immediateVoidFuture()
    }

    override fun handleSeek(mediaItemIndex: Int, positionMs: Long, seekCommand: Int): ListenableFuture<*> {
        when (seekCommand) {
            COMMAND_SEEK_TO_NEXT, COMMAND_SEEK_TO_NEXT_MEDIA_ITEM -> app.core.next()
            COMMAND_SEEK_TO_PREVIOUS, COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM -> app.core.prev()
            else -> app.core.seek(positionMs.coerceAtLeast(0).toULong())
        }
        return Futures.immediateVoidFuture()
    }

    override fun handleStop(): ListenableFuture<*> {
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
