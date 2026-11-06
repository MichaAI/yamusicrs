package xyz.neodustry.yamusicrs

import android.app.PendingIntent
import android.content.Intent
import android.os.Bundle
import androidx.media3.session.CommandButton
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService
import androidx.media3.session.SessionCommand
import androidx.media3.session.SessionResult
import com.google.common.util.concurrent.Futures
import com.google.common.util.concurrent.ListenableFuture
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

/** Медиасессия: уведомление, экран блокировки, гарнитура; держит процесс, пока играет. */
class PlaybackService : MediaSessionService() {
    private var session: MediaSession? = null
    private val scope = MainScope()

    private val like = SessionCommand("xyz.neodustry.yamusicrs.LIKE", Bundle.EMPTY)
    private val dislike = SessionCommand("xyz.neodustry.yamusicrs.DISLIKE", Bundle.EMPTY)

    /** «Не рекомендовать» и «Нравится» (в таком порядке, как в плеере) — в уведомлении рядом с переключением треков. */
    private fun buttons(liked: Boolean): List<CommandButton> = listOf(
        CommandButton.Builder(CommandButton.ICON_UNDEFINED)
            .setCustomIconResId(R.drawable.ic_dislike)
            .setDisplayName("Не рекомендовать")
            .setSessionCommand(dislike)
            .setSlots(CommandButton.SLOT_OVERFLOW)
            .build(),
        CommandButton.Builder(if (liked) CommandButton.ICON_HEART_FILLED else CommandButton.ICON_HEART_UNFILLED)
            .setDisplayName(if (liked) "Убрать из «Мне нравится»" else "Нравится")
            .setSessionCommand(like)
            .setSlots(CommandButton.SLOT_OVERFLOW)
            .build(),
    )

    override fun onCreate() {
        super.onCreate()
        val app = application as YaApp
        val player = CorePlayer(app)
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val liked = { app.state.value?.liked == true }
        val s = MediaSession.Builder(this, player)
            .setSessionActivity(open)
            .setMediaButtonPreferences(buttons(liked()))
            .setCallback(object : MediaSession.Callback {
                override fun onConnect(session: MediaSession, controller: MediaSession.ControllerInfo): MediaSession.ConnectionResult =
                    MediaSession.ConnectionResult.AcceptedResultBuilder(session)
                        .setAvailableSessionCommands(
                            MediaSession.ConnectionResult.DEFAULT_SESSION_COMMANDS.buildUpon().add(like).add(dislike).build(),
                        )
                        .setMediaButtonPreferences(buttons(liked()))
                        .build()

                override fun onCustomCommand(
                    session: MediaSession,
                    controller: MediaSession.ControllerInfo,
                    customCommand: SessionCommand,
                    args: Bundle,
                ): ListenableFuture<SessionResult> {
                    when (customCommand.customAction) {
                        like.customAction -> app.core.toggleLike()
                        dislike.customAction -> app.core.dislike()
                    }
                    return Futures.immediateFuture(SessionResult(SessionResult.RESULT_SUCCESS))
                }
            })
            .build()
        session = s
        // Сердечко в уведомлении следует за лайком.
        scope.launch {
            app.state.map { it?.liked == true }.distinctUntilChanged().collect { s.setMediaButtonPreferences(buttons(it)) }
        }
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? = session

    override fun onTaskRemoved(rootIntent: Intent?) {
        // Смахнули из недавних на паузе — уходим; играет — продолжаем в фоне.
        val p = session?.player
        if (p == null || !p.playWhenReady || p.mediaItemCount == 0) stopSelf()
    }

    override fun onDestroy() {
        scope.cancel()
        session?.run {
            player.release()
            release()
        }
        session = null
        super.onDestroy()
    }
}
