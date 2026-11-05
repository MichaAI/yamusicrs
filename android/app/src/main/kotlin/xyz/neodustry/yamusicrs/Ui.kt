@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package xyz.neodustry.yamusicrs

import androidx.activity.compose.PredictiveBackHandler
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.animation.AnimatedVisibilityScope
import androidx.compose.animation.ExperimentalSharedTransitionApi
import androidx.compose.animation.SharedTransitionLayout
import androidx.compose.animation.SharedTransitionScope
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.PagerDefaults
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.foundation.MutatePriority
import androidx.compose.animation.core.SeekableTransitionState
import androidx.compose.animation.core.rememberTransition
import androidx.compose.ui.input.nestedscroll.nestedScroll
import kotlinx.coroutines.flow.first
import androidx.compose.foundation.gestures.stopScroll
import androidx.compose.runtime.snapshotFlow
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.graphics.Shadow
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import kotlin.math.roundToInt
import kotlinx.coroutines.launch
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.basicMarquee
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.requiredWidth
import androidx.compose.foundation.layout.requiredSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialExpressiveTheme
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MotionScheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.ShortNavigationBar
import androidx.compose.material3.ShortNavigationBarItem
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.Typography
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontVariation
import androidx.compose.material3.TextButton
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.toShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import uniffi.yamusic_android.LyricsData
import uniffi.yamusic_android.Phase
import uniffi.yamusic_android.PlayerState
import uniffi.yamusic_android.TrackItem
import uniffi.yamusic_android.WavePreset

/** Nunito (OFL): округлый гротеск с кириллицей — один шрифт на всё, рифмуется с блобом и формами MD3E. */
private val Rounded = FontFamily(
    listOf(400, 500, 600, 700, 800, 900, 1000).map { w ->
        Font(R.font.nunito, FontWeight(w), variationSettings = FontVariation.Settings(FontVariation.weight(w)))
    },
)

private fun typography(): Typography {
    val t = Typography()
    // TextMotion.Animated: без привязки глифов к пиксельной сетке — текст не дрожит,
    // когда его двигают дробными шагами (бегущая строка, свайпы, сворачивание сцены).
    fun TextStyle.r(tracking: Float = letterSpacing.value) =
        copy(fontFamily = Rounded, letterSpacing = tracking.sp, textMotion = androidx.compose.ui.text.style.TextMotion.Animated)
    return t.copy(
        displayLarge = t.displayLarge.r(-1f),
        displayMedium = t.displayMedium.r(-0.6f),
        displaySmall = t.displaySmall.r(-0.3f),
        headlineLarge = t.headlineLarge.r(-0.3f),
        headlineMedium = t.headlineMedium.r(-0.2f),
        headlineSmall = t.headlineSmall.r(),
        titleLarge = t.titleLarge.r(),
        titleMedium = t.titleMedium.r(),
        titleSmall = t.titleSmall.r(),
        bodyLarge = t.bodyLarge.r(),
        bodyMedium = t.bodyMedium.r(),
        bodySmall = t.bodySmall.r(),
        labelLarge = t.labelLarge.r(),
        labelMedium = t.labelMedium.r(),
        labelSmall = t.labelSmall.r(),
    )
}

// ── тема ─────────────────────────────────────────────────────────────────────

/** Схема MD3 из палитры обложки (как тема TUI): акценты, тонированные поверхности, текст. */
private fun scheme(p: Pal): ColorScheme {
    // Фон — сама обложка, затемнённая (а не нейтральный почти-чёрный).
    val bg = lerp(Color.Black, p.surface, 0.6f)
    val ink = lerp(Color.Black, p.surface, 0.4f)
    return darkColorScheme(
        primary = p.accent,
        onPrimary = ink,
        primaryContainer = lerp(bg, p.accent, 0.32f),
        onPrimaryContainer = Color.White,
        secondary = p.accent2,
        onSecondary = ink,
        secondaryContainer = lerp(bg, p.accent2, 0.28f),
        onSecondaryContainer = Color.White,
        tertiary = p.blob[2],
        background = bg,
        onBackground = lerp(Color(0xFFE8E8E8), p.accent, 0.08f),
        surface = bg,
        onSurface = lerp(Color(0xFFE8E8E8), p.accent, 0.08f),
        onSurfaceVariant = lerp(Color(0xFF8C8C8C), p.accent, 0.15f),
        surfaceContainerLowest = lerp(bg, Color.Black, 0.5f),
        surfaceContainerLow = lerp(bg, p.surface, 0.5f),
        surfaceContainer = lerp(bg, lerp(p.surface, Color.White, 0.04f), 0.8f),
        surfaceContainerHigh = lerp(p.surface, Color.White, 0.07f),
        surfaceContainerHighest = lerp(p.surface, Color.White, 0.12f),
        outline = lerp(Color(0xFF5A5A5A), p.accent, 0.2f),
        outlineVariant = lerp(Color(0xFF333333), p.accent, 0.12f),
    )
}

/**
 * Тема перетекает к новой обложке за 0,9 с — как ThemeAnim в TUI, но ступенями (15 шагов):
 * каждая смена цветов перекомпоновывает и перерисовывает почти весь экран, и плавная анимация
 * на 120 Гц делала из переключения трека ~108 полных кадров. Блоб меняет цвета плавно сам.
 */
@Composable
fun YaTheme(pal: Pal, content: @Composable () -> Unit) {
    // Схема — по палитре: новый объект на каждую перекомпоновку перезапускал бы анимацию.
    val target = remember(pal) { scheme(pal) }
    var from by remember { mutableStateOf(target) }
    var to by remember { mutableStateOf(target) }
    val k = remember { Animatable(1f) }
    // Шаг анимации, а не её значение: перекомпоновка — только когда шаг сменился.
    val step by remember { derivedStateOf { (k.value * STEPS).roundToInt() } }
    LaunchedEffect(target) {
        if (target === to) return@LaunchedEffect
        from = lerpScheme(from, to, step / STEPS.toFloat())
        to = target
        k.snapTo(0f)
        k.animateTo(1f, tween(900))
    }
    val s = remember(from, to, step) { lerpScheme(from, to, step / STEPS.toFloat()) }
    MaterialExpressiveTheme(
        colorScheme = s,
        motionScheme = MotionScheme.expressive(),
        typography = remember { typography() },
        content = content,
    )
}

private const val STEPS = 15

private fun lerpScheme(a: ColorScheme, b: ColorScheme, f: Float): ColorScheme {
    if (f >= 1f) return b
    if (f <= 0f) return a
    fun l(x: Color, y: Color) = lerp(x, y, f)
    return b.copy(
        primary = l(a.primary, b.primary),
        onPrimary = l(a.onPrimary, b.onPrimary),
        primaryContainer = l(a.primaryContainer, b.primaryContainer),
        secondary = l(a.secondary, b.secondary),
        onSecondary = l(a.onSecondary, b.onSecondary),
        secondaryContainer = l(a.secondaryContainer, b.secondaryContainer),
        tertiary = l(a.tertiary, b.tertiary),
        background = l(a.background, b.background),
        onBackground = l(a.onBackground, b.onBackground),
        surface = l(a.surface, b.surface),
        onSurface = l(a.onSurface, b.onSurface),
        onSurfaceVariant = l(a.onSurfaceVariant, b.onSurfaceVariant),
        surfaceContainerLowest = l(a.surfaceContainerLowest, b.surfaceContainerLowest),
        surfaceContainerLow = l(a.surfaceContainerLow, b.surfaceContainerLow),
        surfaceContainer = l(a.surfaceContainer, b.surfaceContainer),
        surfaceContainerHigh = l(a.surfaceContainerHigh, b.surfaceContainerHigh),
        surfaceContainerHighest = l(a.surfaceContainerHighest, b.surfaceContainerHighest),
        outline = l(a.outline, b.outline),
        outlineVariant = l(a.outlineVariant, b.outlineVariant),
    )
}

// ── корень ───────────────────────────────────────────────────────────────────

@Composable
fun AppScreen(app: YaApp, openUrl: (String) -> Unit) {
    val state by app.state.collectAsStateWithLifecycle()
    val pal by app.palette.collectAsStateWithLifecycle()
    YaTheme(pal) {
        // Surface задаёт и цвет текста по умолчанию (иначе он чёрный — чёрным по тёмному).
        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background, contentColor = MaterialTheme.colorScheme.onSurface) {
        Box(Modifier.fillMaxSize()) {
            when (val p = state?.phase) {
                null, is Phase.Connecting -> Connecting(pal, (p as? Phase.Connecting)?.error)
                is Phase.Login -> Login(app, pal, p, openUrl)
                is Phase.Main -> Main(app, pal, state!!)
            }
        }
        }
    }
}

@Composable
private fun Connecting(pal: Pal, error: String?) {
    Box(Modifier.fillMaxSize()) {
        Blob(pal.blob, playing = false, modifier = Modifier.fillMaxSize())
        Column(
            Modifier.fillMaxSize().padding(32.dp),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            LoadingIndicator(Modifier.size(72.dp))
            Spacer(Modifier.height(20.dp))
            Text("Подключаюсь…", style = MaterialTheme.typography.titleLarge)
            error?.let {
                Spacer(Modifier.height(8.dp))
                Text(it, color = MaterialTheme.colorScheme.error, textAlign = TextAlign.Center, style = MaterialTheme.typography.bodySmall)
            }
        }
    }
}

@Composable
private fun Login(app: YaApp, pal: Pal, p: Phase.Login, openUrl: (String) -> Unit) {
    var token by rememberSaveable { mutableStateOf("") }
    Box(Modifier.fillMaxSize()) {
        Blob(pal.blob, playing = false, modifier = Modifier.fillMaxSize())
        Column(
            Modifier.fillMaxSize().systemBarsPadding().padding(24.dp),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Surface(
                shape = RoundedCornerShape(36.dp),
                color = MaterialTheme.colorScheme.surfaceContainer.copy(alpha = 0.86f),
                modifier = Modifier.widthIn(max = 480.dp),
            ) {
                Column(Modifier.padding(28.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    Text("Вход", style = MaterialTheme.typography.displaySmall, fontWeight = FontWeight.Bold)
                    Spacer(Modifier.height(16.dp))
                    val code = p.code
                    val url = p.url
                    if (code != null && url != null) {
                        Text("Откройте ${url.removePrefix("https://")} и введите код", textAlign = TextAlign.Center)
                        Spacer(Modifier.height(16.dp))
                        Box(
                            Modifier.clip(RoundedCornerShape(24.dp)).background(MaterialTheme.colorScheme.primaryContainer)
                                .padding(horizontal = 24.dp, vertical = 12.dp),
                        ) {
                            ScrambleText(
                                code,
                                MaterialTheme.typography.displaySmall.copy(fontWeight = FontWeight.Black, letterSpacing = 6.sp),
                                MaterialTheme.colorScheme.primary,
                            )
                        }
                        Spacer(Modifier.height(16.dp))
                        Button(onClick = { openUrl(url) }) { Text("Открыть страницу входа") }
                    } else {
                        LoadingIndicator()
                        Text(p.status)
                    }
                    p.error?.let {
                        Spacer(Modifier.height(8.dp))
                        Text(it, color = MaterialTheme.colorScheme.error, textAlign = TextAlign.Center)
                    }
                    TextButton(onClick = { app.core.newLoginCode() }) { Text("Новый код") }
                    Spacer(Modifier.height(8.dp))
                    OutlinedTextField(
                        value = token,
                        onValueChange = { token = it },
                        label = { Text("…или токен") },
                        singleLine = true,
                        shape = RoundedCornerShape(20.dp),
                        modifier = Modifier.fillMaxWidth(),
                    )
                    TextButton(onClick = { app.core.submitToken(token.trim()) }, enabled = token.isNotBlank()) {
                        Text("Войти по токену")
                    }
                }
            }
        }
    }
}

// ── главный экран ────────────────────────────────────────────────────────────

private enum class Tab(val title: String) {
    Wave("Волна"), Library("Коллекция"), Search("Поиск"), Releases("Новинки"), Settings("Настройки"),
}

/**
 * Горизонтальный список внутри вкладки, докрученный до края, не отдаёт остаток свайпа
 * пейджеру вкладок — иначе в конце списка начинает листаться экран.
 */
private object KeepHorizontal : androidx.compose.ui.input.nestedscroll.NestedScrollConnection {
    override fun onPostScroll(consumed: Offset, available: Offset, source: androidx.compose.ui.input.nestedscroll.NestedScrollSource) =
        Offset(available.x, 0f)

    override suspend fun onPostFling(consumed: androidx.compose.ui.unit.Velocity, available: androidx.compose.ui.unit.Velocity) =
        androidx.compose.ui.unit.Velocity(available.x, 0f)
}

/** Затухание к концу перехода: пока жест «назад» в начале, уходящее ещё видно. */
val LateFade = androidx.compose.animation.core.Easing { f -> (f * 1.6f - 0.6f).coerceIn(0f, 1f) }

/** Ключи общих элементов: мини-плеер вырастает в сцену, обложка перелетает. */
private const val KEY_PLAYER = "player"
private const val KEY_COVER = "cover"

@OptIn(ExperimentalSharedTransitionApi::class)
@Composable
private fun Main(app: YaApp, pal: Pal, s: PlayerState) {
    val pager = rememberPagerState { Tab.entries.size }
    val scope = rememberCoroutineScope()
    val nav = remember { Nav(app) }
    // Со вкладки, кроме первой, «назад» ведёт на «Волну», а не закрывает приложение.
    androidx.activity.compose.BackHandler(!nav.stage && nav.stack.isEmpty() && pager.currentPage != 0) {
        scope.launch { pager.animateScrollToPage(0) }
    }
    // Позже объявленный обработчик главнее: сначала сворачивается сцена, потом страницы.
    // «Назад» со страниц и сцены — предиктивный (PredictiveBackHandler в PageHost и Stage).
    val clock = rememberPlayClock(s)
    val hasTrack = s.track != null
    // Ушли со строки поиска (страница, сцена, другая вкладка, меню трека) — клавиатура прячется.
    val focus = androidx.compose.ui.platform.LocalFocusManager.current
    val keyboard = androidx.compose.ui.platform.LocalSoftwareKeyboardController.current
    LaunchedEffect(nav.stack.size, nav.stage, pager.currentPage, nav.menu != null, nav.devices, nav.queue, nav.artists != null) {
        focus.clearFocus(force = true)
        keyboard?.hide()
    }
    // Сцена ⇄ мини-плеер — один переход, который «назад» перематывает за пальцем:
    // сцена по ходу жеста сжимается в пилюлю, отпустили — доезжает, передумали — раскрывается.
    val stageState = remember { SeekableTransitionState(false) }
    val stageTr = rememberTransition(stageState, label = "stage")
    LaunchedEffect(nav.stage) {
        if (stageState.currentState != nav.stage || stageState.targetState != nav.stage) stageState.animateTo(nav.stage)
    }
    // «Назад» со сцены — предиктивный: сцену за пальцем сжимает сама Stage (PredictiveBackHandler в ней).
    val now = NowMark(s.track?.id, !s.paused)
    // Непрозрачное поверх блобов: у экрана волны и у сцены — свои.
    val waveHoles = remember { BlobHoles() }
    val stageHoles = remember { BlobHoles() }
    CompositionLocalProvider(LocalNav provides nav, LocalNow provides now) {
    SharedTransitionLayout(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            Box(Modifier.weight(1f)) {
                // Вкладки листаются свайпом вбок.
                // Вкладка переключается уже с пятой части ширины, а не с половины.
                // На главном с играющим треком свайп вбок листает треки, а не вкладки.
                HorizontalPager(
                    pager,
                    key = { it },
                    userScrollEnabled = !(pager.settledPage == Tab.Wave.ordinal && hasTrack && !s.waveLoading),
                    flingBehavior = PagerDefaults.flingBehavior(pager, snapPositionalThreshold = 0.2f),
                ) { page ->
                    Box(Modifier.fillMaxSize().nestedScroll(KeepHorizontal)) {
                    when (Tab.entries[page]) {
                        Tab.Wave -> WaveTab(app, pal, s, waveHoles)
                        Tab.Library -> LibraryTab(app, nav, s.likesLoading)
                        Tab.Search -> SearchTab(app, nav)
                        Tab.Releases -> ReleasesTab(app, nav)
                        Tab.Settings -> SettingsTab(app)
                    }
                    }
                }
                PageHost(app, nav)
                val remote = s.remote
                androidx.compose.animation.AnimatedVisibility(
                    remote != null && !nav.stage,
                    Modifier.align(Alignment.BottomCenter),
                    enter = fadeIn(),
                    exit = fadeOut(),
                ) {
                    remote?.let { RemotePill(app, nav, it) }
                }
                stageTr.AnimatedVisibility(
                    { !it && hasTrack && remote == null },
                    Modifier.align(Alignment.BottomCenter),
                    enter = fadeIn(),
                    exit = fadeOut(),
                ) {
                    MiniPlayer(app, s, clock, this@SharedTransitionLayout, this, waveHoles) { nav.stage = true }
                }
            }
            ShortNavigationBar(containerColor = MaterialTheme.colorScheme.surfaceContainerLow) {
                Tab.entries.forEachIndexed { i, t ->
                    ShortNavigationBarItem(
                        selected = pager.currentPage == i,
                        onClick = {
                            nav.clear()
                            scope.launch { pager.animateScrollToPage(i) }
                        },
                        icon = {
                            Icon(
                                when (t) {
                                    Tab.Wave -> Ic.Wave
                                    Tab.Library -> Ic.Library
                                    Tab.Search -> Ic.Search
                                    Tab.Releases -> Ic.Releases
                                    Tab.Settings -> Ic.Settings
                                },
                                null,
                            )
                        },
                        label = { Text(t.title) },
                    )
                }
            }
        }
        stageTr.AnimatedVisibility({ it && hasTrack }, enter = fadeIn(), exit = fadeOut(tween(easing = LateFade))) {
            Stage(app, pal, s, clock, this@SharedTransitionLayout, this, stageHoles) { nav.stage = false }
        }
    }
    QueueSheet(app, nav, s.queuePos?.toInt())
    DevicesSheet(app, nav, s)
    TrackMenu(app, nav)
    ArtistPicker(nav)
    LyricsSheet(app, nav)
    }
}

// ── Моя волна ────────────────────────────────────────────────────────────────

@Composable
private fun WaveTab(app: YaApp, pal: Pal, s: PlayerState, holes: BlobHoles) {
    val presets by app.wheel.collectAsStateWithLifecycle()
    val waveOn = s.wave
    // Играет что угодно (волна, «Мне нравится», плейлист) — главный экран показывает это потоком.
    val playing = s.track != null && !s.paused
    Box(Modifier.fillMaxSize()) {
        Blob(pal.blob, playing, Modifier.fillMaxSize(), holes = holes)
        Box(
            Modifier.fillMaxSize().background(
                Brush.verticalGradient(0.55f to Color.Transparent, 1f to MaterialTheme.colorScheme.background),
            ),
        )
        Column(Modifier.fillMaxSize().statusBarsPadding().padding(top = 24.dp, bottom = 120.dp)) {
            val now = s.track
            if (now != null && !s.waveLoading) {
                WaveNow(app, s, now, Modifier.weight(1f), holes)
            } else {
            Column(Modifier.padding(horizontal = 24.dp)) {
                ScrambleText(
                    if (waveOn) s.waveName.ifEmpty { "Моя волна" } else "Моя волна",
                    MaterialTheme.typography.displayMedium.copy(fontWeight = FontWeight.Black),
                    MaterialTheme.colorScheme.onSurface,
                )
                val t = s.track
                when {
                    s.waveLoading -> Sub("Подбираю треки")
                    waveOn && t != null -> {
                        Sub(t.title, MaterialTheme.colorScheme.onSurface)
                        Sub(t.artists)
                    }
                    s.offline -> Sub("Без сети: играет из скачанного")
                    else -> Sub("Бесконечный поток под ваш вкус")
                }
            }
            Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                if (s.waveLoading) {
                    LoadingIndicator(Modifier.size(120.dp))
                } else {
                    PlayButton(playing, 132.dp, halo = true, onClick = {
                        if (waveOn && s.track != null) app.core.playPause() else app.core.startWave()
                    })
                }
            }
            }
            if (presets.isNotEmpty()) {
                LazyRow(
                    contentPadding = PaddingValues(horizontal = 20.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    items(presets, key = { it.seeds.joinToString() }) { p ->
                        PresetOrb(app, p, active = waveOn && s.waveSeeds == p.seeds) {
                            app.core.startWaveWith(p.seeds, p.name)
                        }
                    }
                }
            }
        }
    }
}

/**
 * Что-то играет (волна или любой список): в центре — исполнитель (имя крупно, его фото), обложка трека поверх фото,
 * под ними пауза, название трека (тап — сцена) и «нравится».
 */
@Composable
private fun WaveNow(app: YaApp, s: PlayerState, t: TrackItem, modifier: Modifier, holes: BlobHoles) {
    val card by app.artist.collectAsStateWithLifecycle()
    val artist = card?.takeIf { it.artistId == t.artistId }
    val nav = LocalNav.current
    // Свайп вбок по всему экрану — соседний трек: фото с обложкой уезжают за пальцем, улетают и возвращаются новыми.
    val scope = rememberCoroutineScope()
    val dx = remember { Animatable(0f) }
    val skip = with(LocalDensity.current) { 56.dp.toPx() }
    val (canNext, canPrev) = skipBounds(app, s)
    val still by remember { derivedStateOf { dx.value == 0f } }
    Column(
        modifier.fillMaxWidth().pointerInput(Unit) {
            detectHorizontalDragGestures(
                onDragEnd = {
                    scope.launch {
                        val w = size.width.toFloat()
                        when {
                            dx.value < -skip && canNext.value -> {
                                app.core.next()
                                dx.animateTo(-w, tween(160))
                                dx.snapTo(w * 0.4f)
                                dx.animateTo(0f, spring(dampingRatio = 0.8f))
                            }
                            dx.value > skip && canPrev.value -> {
                                app.core.prevTrack()
                                dx.animateTo(w, tween(160))
                                dx.snapTo(-w * 0.4f)
                                dx.animateTo(0f, spring(dampingRatio = 0.8f))
                            }
                            else -> dx.animateTo(0f, spring(dampingRatio = 0.7f))
                        }
                    }
                },
                onDragCancel = { scope.launch { dx.animateTo(0f) } },
            ) { change, d ->
                change.consume()
                scope.launch { dx.snapTo(dx.value + d) }
            }
        },
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        // Что играет (волна или список) — тихой подписью, главное на экране — исполнитель.
        Text(
            if (s.wave) s.waveName.ifEmpty { "Моя волна" } else s.queueTitle.ifEmpty { "Сейчас играет" },
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.Bold,
            color = MaterialTheme.colorScheme.primary,
        )
        ScrambleText(
            artist?.name ?: t.artists,
            MaterialTheme.typography.displayMedium.copy(fontWeight = FontWeight.Black, textAlign = TextAlign.Center),
            MaterialTheme.colorScheme.onSurface,
            Modifier.padding(horizontal = 24.dp).basicMarquee(initialDelayMillis = 2000),
        )
        BoxWithConstraints(
            Modifier.weight(1f).fillMaxWidth().padding(vertical = 12.dp).graphicsLayer {
                translationX = dx.value
                rotationZ = dx.value / size.width * 5f
                alpha = 1f - (kotlin.math.abs(dx.value) / size.width).coerceIn(0f, 1f) * 0.6f
            },
            contentAlignment = Alignment.Center,
        ) {
            val side = minOf(maxWidth - 64.dp, maxHeight - 48.dp, 360.dp).coerceAtLeast(120.dp)
            val small = (side * 0.34f).coerceIn(72.dp, 120.dp)
            Box(Modifier.size(side, side + small / 2), contentAlignment = Alignment.TopCenter) {
                val cutout = artist?.cutout
                if (cutout != null) {
                    // Вырезанное фото — без рамки, прямо на блобе; низ растворяется к обложке.
                    // Вырезка шире квадрата — на всю ширину экрана, низом к обложке, без обрезки.
                    Art(
                        app, cutout,
                        Modifier.requiredSize(this@BoxWithConstraints.maxWidth, side).graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }
                            .drawWithContent {
                                drawContent()
                                drawRect(
                                    Brush.verticalGradient(0.7f to Color.Black, 1f to Color.Transparent),
                                    blendMode = BlendMode.DstIn,
                                )
                            },
                        plain = true,
                        align = Alignment.BottomCenter,
                    )
                } else {
                    // Фото исполнителя (или обложка, если фото нет); концентрично: 48 снаружи.
                    Art(app, artist?.photo ?: t.cover, Modifier.size(side).blobHole(holes, 48.dp, still), corner = 48.dp)
                }
                // Обложка трека наполовину на фото, в ободке цвета фона — читается как отдельный слой.
                // Фото нет — крупно и так обложка, второй раз её не показываем.
                if (cutout != null || artist?.photo != null) Box(
                    Modifier.align(Alignment.BottomCenter).size(small + 8.dp).blobHole(holes, 28.dp, still).clip(RoundedCornerShape(28.dp))
                        .background(MaterialTheme.colorScheme.background).padding(4.dp),
                ) {
                    Art(app, t.cover, Modifier.fillMaxSize(), corner = 24.dp)
                }
            }
        }
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 24.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            PlayButton(!s.paused, 64.dp, onClick = { app.core.playPause() })
            // Название трека — пилюлей той же высоты; тап открывает сцену.
            Column(
                Modifier.weight(1f).height(64.dp).clip(RoundedCornerShape(32.dp))
                    .background(MaterialTheme.colorScheme.surfaceContainerHigh.copy(alpha = 0.8f))
                    .clickable { nav.stage = true }.padding(horizontal = 20.dp),
                verticalArrangement = Arrangement.Center,
            ) {
                Text(t.title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold, maxLines = 1,
                    modifier = Modifier.basicMarquee(initialDelayMillis = 2000))
                if (artist != null && t.album.isNotEmpty()) {
                    Text(t.album, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
            ShapeIconButton(
                if (s.liked) Ic.Heart else Ic.HeartOutline,
                "Нравится",
                { app.core.toggleLike() },
                size = 64.dp,
                container = MaterialTheme.colorScheme.surfaceContainerHigh.copy(alpha = 0.8f),
                tint = if (s.liked) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface,
            )
        }
        Spacer(Modifier.height(16.dp))
    }
}

/** Есть ли куда листать: в конце списка без «волны после списка» карточка не улетает, а пружинит назад. */
@Composable
private fun skipBounds(app: YaApp, s: PlayerState): Pair<androidx.compose.runtime.State<Boolean>, androidx.compose.runtime.State<Boolean>> {
    val st by app.settings.collectAsStateWithLifecycle()
    val pos = s.queuePos?.toInt()
    val next = s.wave || pos == null || pos + 1 < s.queueLen.toInt() || st?.waveAfterList == true
    val prev = pos == null || pos > 0
    return rememberUpdatedState(next) to rememberUpdatedState(prev)
}

@Composable
private fun Sub(text: String, color: Color = MaterialTheme.colorScheme.onSurfaceVariant) {
    Text(text, color = color, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
}

/** Пресет — иконка агента как есть (без подложки и скругления) и имя; у играющего — полоска. */
@Composable
private fun PresetOrb(app: YaApp, p: WavePreset, active: Boolean, onClick: () -> Unit) {
    val bar by animateFloatAsState(if (active) 1f else 0f, MaterialTheme.motionScheme.defaultSpatialSpec(), label = "bar")
    Column(
        Modifier.width(88.dp).clip(RoundedCornerShape(20.dp)).clickable(onClick = onClick).padding(vertical = 6.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Art(app, p.icon, Modifier.size(76.dp), plain = true)
        Spacer(Modifier.height(8.dp))
        Text(
            p.name,
            style = MaterialTheme.typography.labelLarge,
            color = if (active) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(4.dp))
        Box(
            Modifier.width(28.dp * bar).height(4.dp).clip(RoundedCornerShape(2.dp))
                .background(MaterialTheme.colorScheme.primary),
        )
    }
}

// ── коллекция и очередь ──────────────────────────────────────────────────────

@Composable
private fun LibraryTab(app: YaApp, nav: Nav, loading: Boolean) {
    var mode by rememberSaveable { mutableIntStateOf(0) }
    Column(Modifier.fillMaxSize().statusBarsPadding()) {
        Header("Коллекция", "") {}
        Segments(listOf("Мне нравится", "Плейлисты"), mode, { mode = it }, Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp))
        if (mode == 0) Likes(app, nav, loading) else PlaylistsGrid(app, nav)
    }
}

@Composable
private fun Likes(app: YaApp, nav: Nav, loading: Boolean) {
    val likes by app.likes.collectAsStateWithLifecycle()
    val now = LocalNow.current
    var filter by rememberSaveable { mutableStateOf("") }
    val shown = remember(likes, filter) {
        val q = filter.trim().lowercase()
        likes.withIndex().filter { (_, t) -> q.isEmpty() || t.title.lowercase().contains(q) || t.artists.lowercase().contains(q) }
    }
    if (likes.isEmpty()) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            if (loading) LoadingIndicator() else Text("Здесь появятся треки, которые вы лайкнете")
        }
        return
    }
    SearchPill(filter, { filter = it }, "Поиск среди ${tracksWord(likes.size)}")
    val list = rememberLazyListState()
    val nowAt = remember(shown, now.id) { shown.indexOfFirst { it.value.id == now.id }.takeIf { it >= 0 } }
    Box(Modifier.fillMaxSize()) {
    LazyColumn(Modifier.fillMaxSize(), state = list, contentPadding = PaddingValues(bottom = 120.dp)) {
        itemsIndexed(shown, key = { _, it -> it.index }, contentType = { _, _ -> "track" }) { _, (i, t) ->
            TrackRow(
                app, t, current = now.id == t.id, playing = now.playing,
                onLongClick = { nav.menu = MenuTarget(t, uniffi.yamusic_android.Source.Likes, i) },
            ) { app.core.playLikes(i.toUInt()) }
        }
    }
    FloatingPlay { app.core.playLikes(0u) }
    ListJumps(app, list, shown.size, 0, nowAt, nowAt?.let { shown[it].value.thumb })
    }
}

@Composable
fun Header(title: String, subtitle: String, action: @Composable () -> Unit) {
    Row(Modifier.fillMaxWidth().padding(start = 24.dp, end = 16.dp, top = 20.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.headlineLarge, fontWeight = FontWeight.Black)
            if (subtitle.isNotEmpty()) {
                Text(subtitle, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
        action()
    }
}

@Composable
fun SearchPill(value: String, onChange: (String) -> Unit, hint: String, onSubmit: (() -> Unit)? = null, floating: Boolean = false) {
    val focus = androidx.compose.ui.platform.LocalFocusManager.current
    // Плавающая (поиск внизу) — с тенью и светлее, чтобы читалась поверх списка.
    val shape = RoundedCornerShape(28.dp)
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp)
            .then(if (floating) Modifier.shadow(10.dp, shape) else Modifier)
            .clip(shape)
            .background(if (floating) MaterialTheme.colorScheme.surfaceContainerHighest else MaterialTheme.colorScheme.surfaceContainerHigh)
            .padding(start = 18.dp, end = 6.dp, top = 4.dp, bottom = 4.dp).height(48.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Ic.Search, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(22.dp))
        Spacer(Modifier.width(12.dp))
        Box(Modifier.weight(1f)) {
            if (value.isEmpty()) Text(hint, color = MaterialTheme.colorScheme.onSurfaceVariant)
            BasicTextField(
                value,
                onChange,
                singleLine = true,
                textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
                cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                keyboardOptions = KeyboardOptions(imeAction = if (onSubmit != null) ImeAction.Search else ImeAction.Done),
                // Искать — клавиатура уходит, видно результаты.
                keyboardActions = KeyboardActions(onSearch = { focus.clearFocus(); onSubmit?.invoke() }),
                modifier = Modifier.fillMaxWidth(),
            )
        }
        // Очистить — одним тапом, без долгого стирания.
        if (value.isNotEmpty()) {
            ShapeIconButton(Ic.Close, "Очистить", { onChange("") }, size = 44.dp, container = Color.Transparent,
                tint = MaterialTheme.colorScheme.onSurfaceVariant)
        } else {
            Spacer(Modifier.width(12.dp))
        }
    }
}

@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
@Composable
fun TrackRow(
    app: YaApp,
    t: TrackItem,
    current: Boolean,
    playing: Boolean,
    dim: Boolean = false,
    /** Место в рейтинге (популярное исполнителя) — цифрой перед обложкой. */
    rank: Int? = null,
    onLongClick: (() -> Unit)? = null,
    onClick: () -> Unit,
) {
    val alpha = when {
        !t.available -> 0.35f
        dim -> 0.55f
        else -> 1f
    }
    val bg = if (current) MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.55f) else Color.Transparent
    Row(
        // Концентрично миниатюре: 14 + 10 (отступ) = 24.
        Modifier.fillMaxWidth().padding(horizontal = 8.dp).clip(RoundedCornerShape(24.dp)).background(bg)
            .combinedClickable(onLongClick = onLongClick, onClick = { if (t.available) onClick() })
            .padding(horizontal = 10.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        rank?.let {
            Text(
                "$it",
                Modifier.width(28.dp),
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Black,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Box(contentAlignment = Alignment.Center) {
            Art(app, t.thumb, Modifier.size(52.dp), corner = if (current) 26.dp else 14.dp)
            if (current) {
                Box(Modifier.size(52.dp).clip(RoundedCornerShape(26.dp)).background(Color.Black.copy(alpha = 0.45f)))
                EqBars(playing, MaterialTheme.colorScheme.primary)
            }
        }
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            Text(
                t.title,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = if (current) FontWeight.Bold else FontWeight.Medium,
                color = (if (current) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface).copy(alpha = alpha),
            )
            Text(
                t.artists,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = alpha),
            )
        }
        Text(time(t.durationMs.toLong()), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

// ── мини-плеер ───────────────────────────────────────────────────────────────

/**
 * Пилюля внизу. Свайп влево/вправо — следующий/предыдущий трек, вверх или тап — сцена
 * (пилюля вырастает в неё, обложка перелетает).
 */
@OptIn(ExperimentalSharedTransitionApi::class)
@Composable
private fun MiniPlayer(
    app: YaApp,
    s: PlayerState,
    clock: PlayClock,
    shared: SharedTransitionScope,
    visibility: AnimatedVisibilityScope,
    holes: BlobHoles,
    open: () -> Unit,
) {
    val t = s.track ?: return
    // Пока проявляется или гаснет (и пока перелетает в сцену) — полупрозрачна, вырезать нельзя.
    val settled = !visibility.transition.isRunning
    val scope = rememberCoroutineScope()
    val dx = remember { Animatable(0f) }
    val density = LocalDensity.current
    val skip = with(density) { 56.dp.toPx() }
    val up = with(density) { 28.dp.toPx() }
    val (canNext, canPrev) = skipBounds(app, s)
    with(shared) {
        Surface(
            shape = RoundedCornerShape(32.dp),
            color = MaterialTheme.colorScheme.surfaceContainerHigh,
            tonalElevation = 6.dp,
            shadowElevation = 12.dp,
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 10.dp)
                .blobHole(holes, 32.dp, settled)
                .sharedBounds(
                    rememberSharedContentState(KEY_PLAYER),
                    visibility,
                    clipInOverlayDuringTransition = OverlayClip(RoundedCornerShape(32.dp)),
                )
                .pointerInput(Unit) {
                    var x = 0f
                    var y = 0f
                    detectDragGestures(
                        onDragStart = { x = 0f; y = 0f },
                        onDragEnd = {
                            scope.launch {
                                when {
                                    y < -up && kotlin.math.abs(y) > kotlin.math.abs(x) -> {
                                        open()
                                        dx.snapTo(0f)
                                    }
                                    x < -skip && canNext.value -> {
                                        app.core.next()
                                        dx.animateTo(-size.width.toFloat(), tween(160))
                                        dx.snapTo(size.width.toFloat() * 0.4f)
                                        dx.animateTo(0f, spring(dampingRatio = 0.8f))
                                    }
                                    x > skip && canPrev.value -> {
                                        app.core.prevTrack()
                                        dx.animateTo(size.width.toFloat(), tween(160))
                                        dx.snapTo(-size.width.toFloat() * 0.4f)
                                        dx.animateTo(0f, spring(dampingRatio = 0.8f))
                                    }
                                    else -> dx.animateTo(0f, spring(dampingRatio = 0.7f))
                                }
                            }
                        },
                        onDragCancel = { scope.launch { dx.animateTo(0f) } },
                    ) { change, d ->
                        change.consume()
                        x += d.x
                        y += d.y
                        scope.launch { dx.snapTo(x) }
                    }
                }
                .clickable(onClick = open),
        ) {
            Box {
                ProgressFill(clock, t.durationMs.toLong(), Modifier.matchParentSize())
                Row(
                    Modifier
                        .padding(8.dp)
                        .graphicsLayer {
                            translationX = dx.value
                            alpha = 1f - (kotlin.math.abs(dx.value) / size.width).coerceIn(0f, 0.7f)
                        },
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    // Концентрично пилюле: 32 − 8 (отступ) = 24.
                    Art(
                        app,
                        t.cover,
                        Modifier
                            .sharedElement(rememberSharedContentState(KEY_COVER), visibility)
                            .size(52.dp),
                        corner = 24.dp,
                    )
                    Spacer(Modifier.width(12.dp))
                    Column(Modifier.weight(1f)) {
                        ScrambleText(
                            t.title,
                            MaterialTheme.typography.titleMedium.copy(fontWeight = FontWeight.Bold),
                            MaterialTheme.colorScheme.onSurface,
                            Modifier.basicMarquee(initialDelayMillis = 2000),
                        )
                        Text(
                            if (s.loading) "Загрузка…" else t.artists,
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    ShapeIconButton(
                        if (s.liked) Ic.Heart else Ic.HeartOutline,
                        "Нравится",
                        { app.core.toggleLike() },
                        size = 48.dp,
                        container = Color.Transparent,
                        tint = if (s.liked) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    PlayButton(!s.paused, 52.dp, onClick = { app.core.playPause() })
                    ShapeIconButton(Ic.Next, "Дальше", { app.core.next() }, size = 48.dp, container = Color.Transparent)
                }
            }
        }
    }
}

/** Заливка прогресса пилюли: позиция читается только при отрисовке. */
@Composable
private fun ProgressFill(clock: PlayClock, durationMs: Long, modifier: Modifier) {
    val fill = MaterialTheme.colorScheme.primary.copy(alpha = 0.14f)
    val dur = durationMs.coerceAtLeast(1).toFloat()
    Box(
        modifier.drawBehind {
            val frac = (clock.pos() / dur).coerceIn(0f, 1f)
            drawRect(fill, size = Size(size.width * frac, size.height))
        },
    )
}

// ── сцена ────────────────────────────────────────────────────────────────────

/**
 * Сцена (как в TUI): блоб, обложка или текст песни, перемотка и кнопки в зоне большого
 * пальца. Тянется за пальцем вниз и сворачивается обратно в пилюлю; обложка свайпается
 * вбок — соседний трек.
 */
@OptIn(ExperimentalSharedTransitionApi::class)
@Composable
private fun Stage(
    app: YaApp,
    pal: Pal,
    s: PlayerState,
    clock: PlayClock,
    shared: SharedTransitionScope,
    visibility: AnimatedVisibilityScope,
    holes: BlobHoles,
    close: () -> Unit,
) {
    val t = s.track ?: return
    var showLyrics by rememberSaveable { mutableStateOf(false) }
    val lyr by app.lyrics.collectAsStateWithLifecycle()
    val lyrics = lyr?.takeIf { it.trackId == t.id }?.lyrics
    val scope = rememberCoroutineScope()
    val dy = remember { Animatable(0f) }
    val density = LocalDensity.current
    // Палец ушёл вниз на столько — сворачиваемся сразу, не дожидаясь, пока отпустят.
    val dismiss = with(density) { 72.dp.toPx() }
    val skip = with(density) { 56.dp.toPx() }
    // Карусель очереди: её листает свайп по всей сцене, а не только по обложке.
    val q by app.queue.collectAsStateWithLifecycle()
    val items = q.second
    val pos = s.queuePos?.toInt()
    val carousel = pos != null && pos in items.indices && items[pos].id == t.id
    val pager = rememberPagerState(initialPage = pos?.coerceIn(0, (items.size - 1).coerceAtLeast(0)) ?: 0) { items.size }
    val lyricsOn = showLyrics && lyrics != null
    // Под текстом песни карусели нет — свайп вбок просто листает треки.
    val pages = carousel && !lyricsOn
    // Жест живёт дольше одной рекомпозиции (волна дописывает очередь посреди свайпа),
    // поэтому читает свежие значения отсюда, а не перезапускается.
    val curPos by rememberUpdatedState(pos)
    val curItems by rememberUpdatedState(items)
    val curPages by rememberUpdatedState(pages)
    val swipe = remember { SwipeState() }
    // Порог — 15% ширины: свайпы лёгкие.
    val fling = PagerDefaults.flingBehavior(pager, snapPositionalThreshold = 0.15f)
    // Трек сменился не свайпом (кнопка, конец трека) — карусель догоняет. Пока палец
    // на сцене или карусель доезжает после свайпа, не вмешиваемся.
    LaunchedEffect(pos, pages, swipe.active) {
        if (!pages || pos == null || swipe.active) return@LaunchedEffect
        if (pager.currentPage == pos && pager.currentPageOffsetFraction == 0f) return@LaunchedEffect
        if (kotlin.math.abs(pager.currentPage - pos) > 3) pager.scrollToPage(pos) else pager.animateScrollToPage(pos)
    }
    val scrim by animateFloatAsState(if (lyricsOn) 0.62f else 0f, tween(400), label = "scrim")
    // Чем дальше утянули, тем сильнее скругление и меньше масштаб — видно, что сворачивается.
    // («Назад» сворачивает сцену в мини-плеер за пальцем — это переход в Main.)
    // Жест «назад» сворачивает сцену за пальцем тем же сжатием, что и свайп вниз.
    val back = remember { Animatable(0f) }
    var backEdge by remember { mutableIntStateOf(androidx.activity.BackEventCompat.EDGE_LEFT) }
    PredictiveBackHandler(enabled = true) { progress ->
        try {
            progress.collect {
                backEdge = it.swipeEdge
                back.snapTo(it.progress)
            }
            close()
        } catch (e: kotlin.coroutines.cancellation.CancellationException) {
            scope.launch { back.animateTo(0f, spring(dampingRatio = 0.8f)) }
            throw e
        }
    }
    val pull = maxOf((dy.value / with(density) { 400.dp.toPx() }).coerceIn(0f, 1f), back.value)
    with(shared) {
        Box(
            Modifier
                .fillMaxSize()
                .offset { IntOffset(0, dy.value.roundToInt()) }
                .sharedBounds(
                    rememberSharedContentState(KEY_PLAYER),
                    visibility,
                    clipInOverlayDuringTransition = OverlayClip(RoundedCornerShape(32.dp)),
                )
                .graphicsLayer {
                    val k = 1f - pull * 0.1f
                    scaleX = k
                    scaleY = k
                    // От какого края тянут «назад» — туда сцена и подаётся.
                    val dir = if (backEdge == androidx.activity.BackEventCompat.EDGE_RIGHT) -1f else 1f
                    translationX = dir * back.value * 24.dp.toPx()
                    shape = RoundedCornerShape((pull * 40).dp)
                    clip = pull > 0f
                }
                .background(MaterialTheme.colorScheme.background)
                .pointerInput(Unit) {
                    // Один жест на всю сцену: вбок — соседний трек (карусель едет за пальцем),
                    // вниз — свернуть. Что уже забрал ребёнок (перемотка, текст песни), не трогаем.
                    awaitEachGesture {
                        val down = awaitFirstDown(requireUnconsumed = false)
                        val vt = VelocityTracker()
                        vt.addPosition(down.uptimeMillis, down.position)
                        var axis = 0
                        var total = Offset.Zero
                        var closed = false
                        var start = pager.currentPage
                        while (true) {
                            val ev = awaitPointerEvent()
                            val ch = ev.changes.firstOrNull { it.id == down.id } ?: break
                            if (!ch.pressed) break
                            if (ch.isConsumed && axis == 0) return@awaitEachGesture
                            val d = ch.positionChange()
                            total += d
                            vt.addPosition(ch.uptimeMillis, ch.position)
                            if (axis == 0 && total.getDistance() > viewConfiguration.touchSlop) {
                                axis = if (kotlin.math.abs(total.x) > kotlin.math.abs(total.y)) 1 else 2
                                if (axis == 1 && curPages) {
                                    // Перехватываем карусель, где бы она ни была: доезд прошлого
                                    // свайпа и догонялка останавливаются, листаем от видимой страницы.
                                    swipe.grab()
                                    scope.launch { pager.stopScroll(MutatePriority.UserInput) }
                                    start = pager.currentPage
                                }
                            }
                            when (axis) {
                                1 -> {
                                    if (curPages) pager.dispatchRawDelta(-d.x)
                                    ch.consume()
                                }
                                2 -> {
                                    ch.consume()
                                    if (!closed) {
                                        // Экран почти не едет за пальцем — сразу сворачивается в пилюлю.
                                        scope.launch { dy.snapTo((total.y * 0.35f).coerceAtLeast(0f)) }
                                        if (total.y > dismiss) {
                                            closed = true
                                            close()
                                        }
                                    }
                                }
                            }
                        }
                        val v = vt.calculateVelocity()
                        when {
                            axis == 1 && curPages -> {
                                // Отпустили — карусель докатывается штатным снапом пейджера со скоростью
                                // пальца (без рывка на стыке), трек меняется, когда она встала.
                                val end = start == curItems.size - 1 && total.x < -skip
                                swipe.settle(scope) {
                                    pager.scroll(MutatePriority.UserInput) { with(fling) { performFling(-v.x) } }
                                    val p = pager.currentPage
                                    when {
                                        // Дальше очереди некуда: у волны это «догрузи ещё».
                                        end && p == start -> app.core.next()
                                        p != start && p != curPos && p in curItems.indices -> {
                                            app.core.playFrom(uniffi.yamusic_android.Source.Queue, p.toUInt())
                                            // Догонялка не должна вернуть карусель, пока ядро не ответило.
                                            kotlinx.coroutines.withTimeoutOrNull(2000) {
                                                snapshotFlow { curPos }.first { it == p }
                                            }
                                        }
                                    }
                                }
                            }
                            axis == 1 && total.x < -skip -> app.core.next()
                            axis == 1 && total.x > skip -> app.core.prevTrack()
                            axis == 2 && !closed && v.y > 1400f -> close()
                            axis == 2 && !closed -> scope.launch { dy.animateTo(0f, spring(dampingRatio = 0.75f)) }
                        }
                        if (axis == 1 && !curPages) swipe.drop()
                    }
                },
        ) {
            Blob(pal.blob, !s.paused, Modifier.fillMaxSize(), holes = holes)
            // Под текстом песни блоб приглушается, иначе строки теряются на ярких пятнах.
            Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background.copy(alpha = scrim)))
            Box(
                Modifier.fillMaxSize().background(
                    Brush.verticalGradient(
                        0f to Color.Transparent,
                        0.5f to Color.Transparent,
                        1f to MaterialTheme.colorScheme.background.copy(alpha = 0.92f),
                    ),
                ),
            )
            Column(Modifier.fillMaxSize().systemBarsPadding().padding(horizontal = 24.dp, vertical = 12.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    ShapeIconButton(Ic.Down, "Свернуть", close, size = 48.dp)
                    Column(Modifier.weight(1f), horizontalAlignment = Alignment.CenterHorizontally) {
                        Text(
                            s.queueTitle.ifEmpty { "Сейчас играет" },
                            style = MaterialTheme.typography.labelLarge,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        if (s.format.isNotEmpty()) {
                            Text(s.format, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.primary)
                        }
                    }
                    // Симметрия со «Свернуть»: заголовок по центру. Остальные действия — внизу, под пальцем.
                    Spacer(Modifier.width(48.dp))
                }
                BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
                    val wide = maxWidth > maxHeight * 1.2f
                    val art: @Composable (Modifier) -> Unit = { m ->
                        BoxWithConstraints(m, contentAlignment = Alignment.Center) {
                            if (lyricsOn && lyrics != null) {
                                LyricsView(app, lyrics, clock)
                            } else {
                                CoverCarousel(app, t, items, pos, pages, pager, maxWidth, maxHeight, shared, visibility, holes)
                            }
                        }
                    }
                    if (wide) {
                        Row(Modifier.fillMaxSize().padding(vertical = 12.dp), horizontalArrangement = Arrangement.spacedBy(32.dp)) {
                            art(Modifier.weight(1f).fillMaxHeight())
                            Column(Modifier.weight(1f).fillMaxHeight(), verticalArrangement = Arrangement.Bottom) {
                                StageInfo(app, s, t, clock, showLyrics, lyrics != null) { showLyrics = !showLyrics }
                            }
                        }
                    } else {
                        // Кнопки — в зоне большого пальца: под ними остаётся заметный запас.
                        Column(Modifier.fillMaxSize()) {
                            art(Modifier.weight(1f).fillMaxWidth().padding(vertical = 16.dp))
                            StageInfo(app, s, t, clock, showLyrics, lyrics != null) { showLyrics = !showLyrics }
                            Spacer(Modifier.height(24.dp))
                        }
                    }
                }
            }
        }
    }
}

/**
 * Свайп по карусели: палец на сцене или карусель доезжает до выбранной страницы.
 * Новый свайп отменяет доезд прошлого; сброс флага — только от последнего.
 */
private class SwipeState {
    var active by mutableStateOf(false)
        private set
    private var job: kotlinx.coroutines.Job? = null

    fun grab() {
        job?.cancel()
        job = null
        active = true
    }

    fun settle(scope: kotlinx.coroutines.CoroutineScope, block: suspend () -> Unit) {
        job?.cancel()
        val j = scope.launch(start = kotlinx.coroutines.CoroutineStart.LAZY) {
            try {
                block()
            } finally {
                if (job === coroutineContext[kotlinx.coroutines.Job]) {
                    job = null
                    active = false
                }
            }
        }
        job = j
        active = true
        j.start()
    }

    /** Свайп кончился без доезда (карусель пропала посреди жеста). */
    fun drop() {
        if (job == null) active = false
    }
}

/**
 * Обложки очереди каруселью: соседние выглядывают по краям, листает её свайп по всей сцене.
 * Если очередь ещё не пришла или не совпадает с играющим — одна обложка со свайпом.
 */
@OptIn(ExperimentalSharedTransitionApi::class)
@Composable
private fun CoverCarousel(
    app: YaApp,
    t: TrackItem,
    items: List<TrackItem>,
    pos: Int?,
    carousel: Boolean,
    pager: androidx.compose.foundation.pager.PagerState,
    width: Dp,
    height: Dp,
    shared: SharedTransitionScope,
    visibility: AnimatedVisibilityScope,
    holes: BlobHoles,
) {
    // Обложка почти во всю ширину; соседние всё равно выглядывают.
    val side = minOf(width * 0.88f, height, 600.dp)
    // Сцена не перелетает из мини-плеера и обратно — обложка на месте и непрозрачна.
    val still = !visibility.transition.isRunning
    if (!carousel || pos == null) {
        SwipeCover(app, t, side, shared, visibility, holes, still)
        return
    }
    // Карусель стоит: у текущей обложки альфа 1 (в движении — меньше, у соседних — всегда меньше).
    val settled by remember(pager) { derivedStateOf { pager.currentPageOffsetFraction == 0f && !pager.isScrollInProgress } }
    // Карусель шире колонки сцены (её отступы по 24) — соседние обложки доходят до края экрана.
    val full = width + 48.dp
    HorizontalPager(
        pager,
        Modifier.requiredWidth(full),
        contentPadding = PaddingValues(horizontal = (full - side) / 2),
        pageSpacing = 10.dp,
        beyondViewportPageCount = 1,
        // Листает её жест сцены — один на всю сцену, с одними порогами.
        userScrollEnabled = false,
        verticalAlignment = Alignment.CenterVertically,
        flingBehavior = PagerDefaults.flingBehavior(pager, snapPositionalThreshold = 0.15f),
    ) { page ->
        val it = items[page]
        Box(
            Modifier.graphicsLayer {
                val off = kotlin.math.abs(pager.currentPage - page + pager.currentPageOffsetFraction).coerceIn(0f, 1f)
                val k = 1f - 0.1f * off
                scaleX = k
                scaleY = k
                alpha = 1f - 0.35f * off
            },
        ) {
            // Одна и та же обложка на странице, меняется только модификатор: когда играющий
            // сменился, обложка не пересоздаётся (иначе мигает), а лишь становится общим элементом.
            val m = if (page == pos) with(shared) { Modifier.sharedElement(rememberSharedContentState(KEY_COVER), visibility) } else Modifier
            Art(app, it.cover, m.size(side).blobHole(holes, 36.dp, still && settled && page == pos && pager.currentPage == page), corner = 36.dp)
        }
    }
}

/** Обложка сцены: свайп вбок — соседний трек (улетает и возвращается уже новой). */
@OptIn(ExperimentalSharedTransitionApi::class)
@Composable
private fun SwipeCover(app: YaApp, t: TrackItem, side: Dp, shared: SharedTransitionScope, visibility: AnimatedVisibilityScope, holes: BlobHoles, still: Boolean) {
    val scope = rememberCoroutineScope()
    val dx = remember { Animatable(0f) }
    val skip = with(LocalDensity.current) { 56.dp.toPx() }
    val flat by remember { derivedStateOf { dx.value == 0f } }
    with(shared) {
        Art(
            app,
            t.cover,
            Modifier
                .sharedElement(rememberSharedContentState(KEY_COVER), visibility)
                .size(side)
                .blobHole(holes, 36.dp, still && flat)
                .graphicsLayer {
                    translationX = dx.value
                    rotationZ = dx.value / size.width * 6f
                }
                .pointerInput(Unit) {
                    detectHorizontalDragGestures(
                        onDragEnd = {
                            scope.launch {
                                val w = size.width.toFloat()
                                when {
                                    dx.value < -skip -> {
                                        app.core.next()
                                        dx.animateTo(-w * 1.2f, tween(180))
                                        dx.snapTo(w * 0.5f)
                                        dx.animateTo(0f, spring(dampingRatio = 0.8f))
                                    }
                                    dx.value > skip -> {
                                        app.core.prevTrack()
                                        dx.animateTo(w * 1.2f, tween(180))
                                        dx.snapTo(-w * 0.5f)
                                        dx.animateTo(0f, spring(dampingRatio = 0.8f))
                                    }
                                    else -> dx.animateTo(0f, spring(dampingRatio = 0.7f))
                                }
                            }
                        },
                    ) { change, d ->
                        change.consume()
                        scope.launch { dx.snapTo(dx.value + d) }
                    }
                },
            corner = 36.dp,
        )
    }
}

/** Название, исполнитель, перемотка и кнопки сцены. */
@Composable
private fun ColumnScope.StageInfo(
    app: YaApp,
    s: PlayerState,
    t: TrackItem,
    clock: PlayClock,
    showLyrics: Boolean,
    hasLyrics: Boolean,
    toggleLyrics: () -> Unit,
) {
    // Длинное название бежит строкой, как marquee в TUI.
    ScrambleText(
        t.title,
        MaterialTheme.typography.headlineMedium.copy(fontWeight = FontWeight.Black),
        MaterialTheme.colorScheme.onSurface,
        Modifier.basicMarquee(initialDelayMillis = 2000),
    )
    val nav = LocalNav.current
    // Исполнитель и альбом — ссылки на свои страницы.
    val card by app.artist.collectAsStateWithLifecycle()
    val photo = card?.takeIf { it.artistId == t.artistId }?.photo
    Row(
        Modifier.clip(RoundedCornerShape(18.dp)).clickable(enabled = t.artistList.isNotEmpty()) { nav.artistOf(t) }
            .padding(vertical = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // Аватарка исполнителя — круглая, как на его странице.
        if (photo != null) {
            Art(app, photo, Modifier.size(32.dp), corner = 16.dp)
            Spacer(Modifier.width(10.dp))
        }
        Text(t.artists, style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSurface,
            maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
    if (t.album.isNotEmpty() && t.album != t.title) {
        Text(t.album, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1, overflow = TextOverflow.Ellipsis,
            modifier = Modifier.clip(RoundedCornerShape(10.dp)).clickable(enabled = t.albumId != null) { nav.album(t.albumId, t.album) })
    }
    Spacer(Modifier.height(12.dp))
    WavySeek(clock, t.durationMs.toLong(), !s.paused && !s.loading, { app.core.seek(it.toULong()) })
    Spacer(Modifier.height(8.dp))
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
        ShapeIconButton(Ic.Dislike, "Не рекомендовать", { app.core.dislike() }, size = 52.dp, container = Color.Transparent,
            tint = MaterialTheme.colorScheme.onSurfaceVariant)
        ShapeIconButton(Ic.Prev, "Назад", { app.core.prev() }, size = 64.dp)
        PlayButton(!s.paused, 96.dp, onClick = { app.core.playPause() })
        ShapeIconButton(Ic.Next, "Дальше", { app.core.next() }, size = 64.dp)
        ShapeIconButton(
            if (s.liked) Ic.Heart else Ic.HeartOutline,
            "Нравится",
            { app.core.toggleLike() },
            size = 52.dp,
            container = Color.Transparent,
            tint = if (s.liked) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
    Spacer(Modifier.height(16.dp))
    // Второстепенное — одним рядом под кнопками, в зоне большого пальца: текст, очередь, устройства, ещё.
    // Кнопки фиксированной ширины, очередь забирает остаток: длинное «128 из 1500» не сдавливает соседей.
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
        val idle = MaterialTheme.colorScheme.surfaceContainerHighest.copy(alpha = 0.7f)
        ShapeIconButton(
            Ic.Lyrics,
            "Текст песни",
            toggleLyrics,
            size = 48.dp,
            container = if (showLyrics && hasLyrics) MaterialTheme.colorScheme.primary else idle,
            // У трека нет текста — кнопка тусклая.
            tint = when {
                !hasLyrics -> MaterialTheme.colorScheme.onSurface.copy(alpha = 0.35f)
                showLyrics -> MaterialTheme.colorScheme.onPrimary
                else -> MaterialTheme.colorScheme.onSurface
            },
        )
        Row(
            Modifier.weight(1f).height(48.dp).clip(RoundedCornerShape(24.dp)).background(idle)
                .clickable { nav.queue = true }.padding(horizontal = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.Center,
        ) {
            Icon(Ic.Queue, "Очередь", Modifier.size(20.dp))
            // Без слова «Очередь» — иконки хватает, и «128 из 1949» помещается целиком.
            if (s.queueLen > 0u) {
                Spacer(Modifier.width(8.dp))
                Text(
                    "${(s.queuePos?.toInt() ?: 0) + 1} из ${s.queueLen}",
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.primary,
                    maxLines = 1,
                    softWrap = false,
                )
            }
        }
        ShapeIconButton(Ic.Cast, "Слушать на", { nav.devices = true }, size = 48.dp, container = idle)
        ShapeIconButton(Ic.More, "Ещё", {
            val i = s.queuePos?.toInt()
            if (i != null) nav.menu = MenuTarget(t, uniffi.yamusic_android.Source.Queue, i)
        }, size = 48.dp, container = idle)
    }
    Spacer(Modifier.height(8.dp))
}

/**
 * Текст песни. Синхронный: строки одного размера (без прыжков вёрстки), текущая — белая и
 * чуть крупнее масштабом, пройденные гаснут, будущие приглушены; держится на трети высоты,
 * тап по строке — перемотка. Обычный — плотным абзацем, пустые строки — паузы между куплетами.
 */
@Composable
private fun LyricsView(app: YaApp, l: LyricsData, clock: PlayClock) {
    val base = MaterialTheme.typography.titleLarge.copy(
        fontWeight = FontWeight.ExtraBold,
        lineHeight = 30.sp,
        shadow = Shadow(Color.Black.copy(alpha = 0.6f), blurRadius = 16f),
    )
    val ink = MaterialTheme.colorScheme.onSurface
    if (!l.synced) {
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(vertical = 16.dp)) {
            l.lines.forEach {
                if (it.text.isBlank()) Spacer(Modifier.height(14.dp))
                else Text(it.text, style = base.copy(fontWeight = FontWeight.Bold, lineHeight = 28.sp), color = ink.copy(alpha = 0.9f))
            }
        }
        return
    }
    val cur by remember(l) { derivedStateOf { l.lines.indexOfLast { it.timeMs.toLong() <= clock.pos() + 250 } } }
    val list = rememberLazyListState()
    LaunchedEffect(cur) { if (cur >= 0) list.animateScrollToItem(cur) }
    BoxWithConstraints(Modifier.fillMaxSize()) {
        LazyColumn(Modifier.fillMaxSize(), state = list, contentPadding = PaddingValues(top = maxHeight / 3, bottom = maxHeight * 2 / 3)) {
            itemsIndexed(l.lines) { i, line ->
                val active = i == cur
                val alpha by animateFloatAsState(
                    when {
                        active -> 1f
                        i < cur -> 0.3f
                        else -> 0.55f
                    },
                    tween(300),
                    label = "line",
                )
                val scale by animateFloatAsState(if (active) 1.06f else 1f, tween(300), label = "scale")
                if (line.text.isBlank()) {
                    // Проигрыш — короткая пауза, а не целая строка с символом.
                    Spacer(Modifier.height(20.dp))
                } else {
                    Text(
                        line.text,
                        style = base,
                        color = ink,
                        modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp))
                            .clickable { app.core.seek(line.timeMs) }
                            .padding(vertical = 5.dp, horizontal = 4.dp)
                            .graphicsLayer {
                                this.alpha = alpha
                                scaleX = scale
                                scaleY = scale
                                transformOrigin = androidx.compose.ui.graphics.TransformOrigin(0f, 0.5f)
                            },
                    )
                }
            }
        }
    }
}
