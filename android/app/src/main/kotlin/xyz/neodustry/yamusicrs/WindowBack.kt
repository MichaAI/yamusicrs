package xyz.neodustry.yamusicrs

import android.app.Dialog
import android.graphics.drawable.ColorDrawable
import android.os.Build
import android.view.Gravity
import android.view.View
import android.view.Window
import android.view.WindowManager
import android.window.BackEvent
import android.window.OnBackAnimationCallback
import android.window.OnBackInvokedDispatcher
import androidx.activity.BackEventCompat
import androidx.activity.compose.LocalActivity
import androidx.activity.compose.PredictiveBackHandler
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.launch
import kotlin.coroutines.cancellation.CancellationException

private const val PROGRESS_POWER = 1.4f

/**
 * Как [PredictiveBackHandler], но колбэк «назад» живёт в отдельном окне (1×1, прозрачное,
 * не ловит касаний), пока обработчик включён.
 *
 * Зачем: на vivo (OriginOS, Android 16) система не стримит прогресс жеста окну самой Activity —
 * приходит started и три progress с p=0, дальше тишина до invoked. Окнам-диалогам тот же
 * OnBackAnimationCallback получает полный поток (p 0→1 с касанием), поэтому и шторки работают,
 * а Telegram показывает предпросмотр в PhotoViewer (отдельное окно). На других оболочках
 * поведение то же, что у AndroidX.
 */
@Composable
fun WindowBackHandler(enabled: Boolean, onBack: suspend (Flow<BackEventCompat>) -> Unit) {
    val activity = LocalActivity.current
    if (Build.VERSION.SDK_INT < 34 || activity == null) {
        PredictiveBackHandler(enabled, onBack)
        return
    }
    val current = rememberUpdatedState(onBack)
    val scope = rememberCoroutineScope()
    DisposableEffect(enabled, activity) {
        if (!enabled) return@DisposableEffect onDispose {}
        val dialog = Dialog(activity)
        dialog.requestWindowFeature(Window.FEATURE_NO_TITLE)
        dialog.setContentView(View(activity))
        dialog.setCancelable(false)
        dialog.window?.apply {
            setBackgroundDrawable(ColorDrawable(0))
            setLayout(1, 1)
            setGravity(Gravity.TOP or Gravity.START)
            setWindowAnimations(0)
            setDimAmount(0f)
            clearFlags(WindowManager.LayoutParams.FLAG_DIM_BEHIND)
            // Касания мимо окна 1×1 уходят приложению. NOT_TOUCHABLE/ALT_FOCUSABLE_IM нельзя:
            // с ними vivo перестаёт слать жест «назад» этому окну.
            addFlags(WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL)
        }
        var events: Channel<BackEventCompat>? = null
        var job: Job? = null
        // vivo разгоняет progress сразу (0.1 после первых пикселей, 0.5 на трети экрана): начало
        // жеста пролетает. Степень замедляет начало и оставляет 1 в конце.
        fun compat(e: BackEvent) =
            BackEventCompat(e.touchX, e.touchY, Math.pow(e.progress.toDouble(), PROGRESS_POWER.toDouble()).toFloat(), e.swipeEdge)
        val callback = object : OnBackAnimationCallback {
            override fun onBackStarted(backEvent: BackEvent) {
                events?.cancel(CancellationException("новый жест"))
                job?.cancel()
                val c = Channel<BackEventCompat>(Channel.UNLIMITED)
                events = c
                c.trySend(compat(backEvent))
                job = scope.launch { current.value(c.receiveAsFlow()) }
            }

            override fun onBackProgressed(backEvent: BackEvent) {
                events?.trySend(compat(backEvent))
            }

            override fun onBackCancelled() {
                // Collector получает CancellationException и сам возвращает интерфейс на место.
                events?.cancel(CancellationException("жест отменён"))
                events = null
            }

            override fun onBackInvoked() {
                events?.close()
                events = null
            }
        }
        dialog.show()
        // После show(): колбэк самого Dialog зарегистрирован раньше, наш с тем же приоритетом главнее.
        dialog.onBackInvokedDispatcher.registerOnBackInvokedCallback(OnBackInvokedDispatcher.PRIORITY_DEFAULT, callback)
        onDispose {
            dialog.onBackInvokedDispatcher.unregisterOnBackInvokedCallback(callback)
            dialog.dismiss()
        }
    }
}
