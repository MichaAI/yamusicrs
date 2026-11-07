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
import androidx.activity.ComponentActivity
import androidx.activity.compose.LocalActivity
import androidx.activity.compose.PredictiveBackHandler
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.LocalWindowInfo
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.launch
import kotlin.coroutines.cancellation.CancellationException

private const val PROGRESS_POWER = 1.4f

/**
 * Сколько текстовых полей сейчас в фокусе. Окно «назад» ([BackHost]) забирает у Activity фокус
 * окна, а без него клавиатура не открывается: пока поле в фокусе, окно спрятано и «назад»
 * идёт через обычный [PredictiveBackHandler].
 */
object TextFocus {
    var count by mutableIntStateOf(0)
}

/**
 * Помечает текстовое поле для [TextFocus]: на время фокуса окно «назад» прячется, а когда
 * Activity вернёт себе фокус окна — клавиатура показывается снова (первая попытка Compose
 * прошла, пока фокус был у чужого окна, и была проигнорирована системой).
 */
@Composable
fun Modifier.trackTextFocus(): Modifier {
    val keyboard = LocalSoftwareKeyboardController.current
    val window = LocalWindowInfo.current
    var focused by remember { mutableStateOf(false) }
    DisposableEffect(focused) {
        val mine = focused // значение на момент запуска: в onDispose state уже новый
        if (mine) TextFocus.count++
        onDispose { if (mine) TextFocus.count-- }
    }
    LaunchedEffect(focused) {
        if (!focused) return@LaunchedEffect
        snapshotFlow { window.isWindowFocused }.first { it }
        delay(50)
        keyboard?.show()
    }
    return onFocusChanged { focused = it.isFocused }
}

/**
 * Одно на всё приложение окно для жеста «назад» (1×1, прозрачное, не ловит касаний).
 *
 * Зачем окно: на vivo (OriginOS, Android 16) система не стримит прогресс жеста окну самой
 * Activity — приходит started и три progress с p=0, дальше тишина до invoked. Окнам-диалогам тот
 * же OnBackAnimationCallback получает полный поток (p 0→1 с касанием), поэтому и шторки работают.
 *
 * Зачем одно: создание окна — binder, relayout и первая отрисовка (~30–50 мс на главном потоке),
 * а раньше оно создавалось и убиралось на каждом переходе (страница, сцена, вкладка), прямо в
 * первом кадре анимации. Теперь окно создаётся один раз заранее ([warmUp]) и дальше только
 * прячется/показывается (смена видимости, без нового окна), а жест получает самый «свежий»
 * включённый обработчик.
 */
class BackHost(private val activity: ComponentActivity) {
    class Entry(val scope: CoroutineScope) {
        var enabled = false
        var stamp = 0L
        var onBack: suspend (Flow<BackEventCompat>) -> Unit = {}
    }

    private val entries = ArrayList<Entry>()
    private var stampCounter = 0L
    private var dialog: Dialog? = null
    private var visible = false
    private var events: Channel<BackEventCompat>? = null
    private var job: Job? = null

    fun add(e: Entry) {
        entries += e
    }

    fun remove(e: Entry) {
        entries -= e
        refresh()
    }

    fun setEnabled(e: Entry, enabled: Boolean) {
        // Позже включённый главнее: сначала сворачивается сцена, потом страницы.
        if (enabled && !e.enabled) e.stamp = ++stampCounter
        e.enabled = enabled
        refresh()
    }

    /** Окно показано, пока есть включённый обработчик и ни одно текстовое поле не в фокусе. */
    fun refresh() {
        val want = TextFocus.count == 0 && entries.any { it.enabled }
        if (want == visible) return
        if (want) {
            val d = ensure()
            d.show()
        } else {
            dialog?.hide()
        }
        visible = want
    }

    /** Создать окно заранее, пока никто не ждёт: потом переходы платят только за смену видимости. */
    fun warmUp() {
        if (Build.VERSION.SDK_INT < 34 || dialog != null || activity.isFinishing || activity.isDestroyed) return
        ensure().hide()
        visible = false
        refresh()
    }

    fun release() {
        events?.cancel(CancellationException("активность закрыта"))
        events = null
        job?.cancel()
        dialog?.dismiss()
        dialog = null
        visible = false
    }

    private fun top(): Entry? = entries.filter { it.enabled }.maxByOrNull { it.stamp }

    private fun compat(e: BackEvent) =
        BackEventCompat(e.touchX, e.touchY, Math.pow(e.progress.toDouble(), PROGRESS_POWER.toDouble()).toFloat(), e.swipeEdge)

    private fun start(first: BackEventCompat): Channel<BackEventCompat>? {
        val entry = top() ?: return null
        events?.cancel(CancellationException("новый жест"))
        job?.cancel()
        val c = Channel<BackEventCompat>(Channel.UNLIMITED)
        events = c
        c.trySend(first)
        job = entry.scope.launch { entry.onBack(c.receiveAsFlow()) }
        return c
    }

    private val callback = object : OnBackAnimationCallback {
        // vivo разгоняет progress сразу (0.1 после первых пикселей, 0.5 на трети экрана): начало
        // жеста пролетает. Степень замедляет начало и оставляет 1 в конце.
        override fun onBackStarted(backEvent: BackEvent) {
            start(compat(backEvent))
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
            val c = events
            if (c != null) {
                c.close()
                events = null
            } else {
                // Кнопка «назад» (3 кнопки) — жеста не было: прогоняем обработчик одним событием.
                start(BackEventCompat(0f, 0f, 0f, BackEvent.EDGE_LEFT))?.close()
            }
        }
    }

    private fun ensure(): Dialog {
        dialog?.let { return it }
        val d = Dialog(activity)
        d.requestWindowFeature(Window.FEATURE_NO_TITLE)
        d.setContentView(View(activity))
        d.setCancelable(false)
        d.window?.apply {
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
        d.show()
        // После show(): колбэк самого Dialog зарегистрирован раньше, наш с тем же приоритетом главнее.
        d.onBackInvokedDispatcher.registerOnBackInvokedCallback(OnBackInvokedDispatcher.PRIORITY_DEFAULT, callback)
        dialog = d
        return d
    }
}

/**
 * Как [PredictiveBackHandler], но жест принимает общее окно [BackHost] (см. там, зачем окно).
 * На других версиях и пока в фокусе текстовое поле — обычный обработчик Activity.
 */
@Composable
fun WindowBackHandler(enabled: Boolean, onBack: suspend (Flow<BackEventCompat>) -> Unit) {
    val activity = LocalActivity.current
    val host = (activity as? MainActivity)?.backHost
    if (Build.VERSION.SDK_INT < 34 || host == null) {
        PredictiveBackHandler(enabled, onBack)
        return
    }
    // Пока поле в фокусе, окно спрятано (иначе оно отбирает фокус окна и клавиатура не
    // открывается) — тогда «назад» идёт через Activity.
    PredictiveBackHandler(enabled && TextFocus.count > 0, onBack)
    val scope = rememberCoroutineScope()
    val entry = remember { BackHost.Entry(scope) }
    val current = rememberUpdatedState(onBack)
    entry.onBack = { current.value(it) }
    DisposableEffect(host) {
        host.add(entry)
        onDispose { host.remove(entry) }
    }
    SideEffect { host.setEnabled(entry, enabled && TextFocus.count == 0) }
}
