package xyz.neodustry.yamusicrs

import android.app.AlarmManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import org.json.JSONObject
import uniffi.yamusic_android.UpcomingItem
import java.util.Calendar

/**
 * Уведомление в день выхода пресейвнутого релиза. На каждый — неточный будильник; сами пресейвы
 * лежат в SharedPreferences, чтобы будильники пережили перезагрузку и обновление приложения.
 * Узнаём о пресейвах из списка в «Новинках», с карточек исполнителей и по нажатию колокольчика.
 */
object ReleaseAlarms {
    const val ACTION = "xyz.neodustry.yamusicrs.RELEASE"
    private const val PREFS = "presaves"
    private const val CHANNEL = "releases"
    /** Сильно опоздавшее (телефон был выключен неделю) уже не новость. */
    private const val STALE_MS = 2L * 24 * 3600 * 1000

    private fun prefs(c: Context) = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /** Релизы выходят в полночь — напоминаем в 10:00 того же дня (по местному времени), не ночью. */
    private fun notifyAt(releaseMs: Long): Long {
        val cal = Calendar.getInstance().apply {
            timeInMillis = releaseMs
            set(Calendar.HOUR_OF_DAY, 10); set(Calendar.MINUTE, 0); set(Calendar.SECOND, 0); set(Calendar.MILLISECOND, 0)
        }
        return maxOf(cal.timeInMillis, releaseMs)
    }

    private fun encode(u: UpcomingItem, at: Long) = JSONObject()
        .put("title", u.title).put("artists", u.artists).put("kind", u.kind).put("at", at).toString()

    /** Пресейв поставлен или снят (в этом приложении или где-то ещё — сервер сказал). */
    fun set(c: Context, u: UpcomingItem, on: Boolean) {
        val at = u.releaseMs?.let(::notifyAt)
        val e = prefs(c).edit()
        if (on && at != null) e.putString(u.id, encode(u, at)) else e.remove(u.id)
        e.apply()
        schedule(c, u.id)
    }

    /** Полный список пресейвов с сервера: чего в нём нет, то отменено. */
    fun sync(c: Context, list: List<UpcomingItem>) {
        val p = prefs(c)
        val keep = list.map { it.id }.toSet()
        val gone = p.all.keys.filter { it !in keep }
        p.edit().apply { gone.forEach(::remove) }.apply()
        gone.forEach { schedule(c, it) }
        list.forEach { set(c, it, true) }
    }

    /** После перезагрузки будильников нет — ставим заново. */
    fun rescheduleAll(c: Context) = prefs(c).all.keys.forEach { schedule(c, it) }

    private fun pending(c: Context, id: String) = PendingIntent.getBroadcast(
        c, id.hashCode(),
        Intent(c, ReleaseReceiver::class.java).setAction(ACTION).putExtra("id", id),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun schedule(c: Context, id: String) {
        val am = c.getSystemService(AlarmManager::class.java) ?: return
        val at = prefs(c).getString(id, null)?.let { JSONObject(it).optLong("at") }
        if (at == null) am.cancel(pending(c, id)) else am.set(AlarmManager.RTC_WAKEUP, at, pending(c, id))
    }

    /** Будильник сработал: уведомление и запись долой. */
    fun fire(c: Context, id: String) {
        val p = prefs(c)
        val j = p.getString(id, null)?.let(::JSONObject) ?: return
        p.edit().remove(id).apply()
        if (System.currentTimeMillis() - j.optLong("at") > STALE_MS) return
        val nm = c.getSystemService(NotificationManager::class.java) ?: return
        nm.createNotificationChannel(NotificationChannel(CHANNEL, "Выход пресейвов", NotificationManager.IMPORTANCE_DEFAULT))
        val open = PendingIntent.getActivity(
            c, 0, Intent(c, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val kind = j.optString("kind").lowercase().ifEmpty { "релиз" }
        val n = NotificationCompat.Builder(c, CHANNEL)
            .setSmallIcon(R.drawable.ic_launcher_mono)
            .setContentTitle(j.optString("title"))
            .setContentText("Вышел $kind: ${j.optString("artists")}. Уже в коллекции")
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        runCatching { NotificationManagerCompat.from(c).notify(id.hashCode(), n) }
    }
}

class ReleaseReceiver : BroadcastReceiver() {
    override fun onReceive(c: Context, i: Intent) {
        when (i.action) {
            ReleaseAlarms.ACTION -> ReleaseAlarms.fire(c, i.getStringExtra("id") ?: return)
            Intent.ACTION_BOOT_COMPLETED, Intent.ACTION_MY_PACKAGE_REPLACED -> ReleaseAlarms.rescheduleAll(c)
        }
    }
}
