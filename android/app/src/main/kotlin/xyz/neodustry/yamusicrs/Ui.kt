@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package xyz.neodustry.yamusicrs

import androidx.activity.compose.BackHandler
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
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.PagerDefaults
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.snapshotFlow
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.graphics.Shadow
import androidx.compose.ui.graphics.graphicsLayer
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
import androidx.compose.material3.Snackbar
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
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

/** Заголовки: Unbounded (OFL) — широкий и округлый, рифмуется с блобом и морфингом форм. */
private val Display = FontFamily(
    listOf(500, 600, 700, 800, 900).map { w ->
        Font(R.font.unbounded, FontWeight(w), variationSettings = FontVariation.Settings(FontVariation.weight(w)))
    },
)

private fun typography(): Typography {
    val t = Typography()
    fun TextStyle.display(tracking: Float) = copy(fontFamily = Display, letterSpacing = tracking.sp)
    return t.copy(
        displayLarge = t.displayLarge.display(-1.5f),
        displayMedium = t.displayMedium.display(-1f),
        displaySmall = t.displaySmall.display(-0.5f),
        headlineLarge = t.headlineLarge.display(-0.5f),
        headlineMedium = t.headlineMedium.display(-0.3f),
        headlineSmall = t.headlineSmall.display(-0.2f),
        titleLarge = t.titleLarge.display(0f),
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

/** Тема плавно перетекает к новой обложке за 0,9 с — как ThemeAnim в TUI. */
@Composable
fun YaTheme(pal: Pal, content: @Composable () -> Unit) {
    val target = scheme(pal)
    @Composable
    fun Color.anim() = animateColorAsState(this, tween(900), label = "theme").value
    val s = target.copy(
        primary = target.primary.anim(),
        primaryContainer = target.primaryContainer.anim(),
        secondary = target.secondary.anim(),
        secondaryContainer = target.secondaryContainer.anim(),
        tertiary = target.tertiary.anim(),
        background = target.background.anim(),
        surface = target.surface.anim(),
        onSurface = target.onSurface.anim(),
        onSurfaceVariant = target.onSurfaceVariant.anim(),
        surfaceContainerLow = target.surfaceContainerLow.anim(),
        surfaceContainer = target.surfaceContainer.anim(),
        surfaceContainerHigh = target.surfaceContainerHigh.anim(),
        surfaceContainerHighest = target.surfaceContainerHighest.anim(),
        outline = target.outline.anim(),
        outlineVariant = target.outlineVariant.anim(),
    )
    MaterialExpressiveTheme(
        colorScheme = s,
        motionScheme = MotionScheme.expressive(),
        typography = remember { typography() },
        content = content,
    )
}

// ── корень ───────────────────────────────────────────────────────────────────

@Composable
fun AppScreen(app: YaApp, openUrl: (String) -> Unit) {
    val state by app.state.collectAsStateWithLifecycle()
    val pal by app.palette.collectAsStateWithLifecycle()
    val snack = remember { SnackbarHostState() }
    LaunchedEffect(Unit) { app.toasts.collect { snack.showSnackbar(it.text) } }
    YaTheme(pal) {
        // Surface задаёт и цвет текста по умолчанию (иначе он чёрный — чёрным по тёмному).
        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background, contentColor = MaterialTheme.colorScheme.onSurface) {
        Box(Modifier.fillMaxSize()) {
            when (val p = state?.phase) {
                null, is Phase.Connecting -> Connecting(pal, (p as? Phase.Connecting)?.error)
                is Phase.Login -> Login(app, pal, p, openUrl)
                is Phase.Main -> Main(app, pal, state!!)
            }
            SnackbarHost(snack, Modifier.align(Alignment.BottomCenter).padding(bottom = 150.dp)) {
                Snackbar(
                    it,
                    shape = RoundedCornerShape(20.dp),
                    containerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                    contentColor = MaterialTheme.colorScheme.onSurface,
                )
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

/** Ключи общих элементов: мини-плеер вырастает в сцену, обложка перелетает. */
private const val KEY_PLAYER = "player"
private const val KEY_COVER = "cover"

@OptIn(ExperimentalSharedTransitionApi::class)
@Composable
private fun Main(app: YaApp, pal: Pal, s: PlayerState) {
    val pager = rememberPagerState { Tab.entries.size }
    val scope = rememberCoroutineScope()
    val nav = remember { Nav(app) }
    // Позже объявленный обработчик главнее: сначала сворачивается сцена, потом страницы.
    BackHandler(nav.stack.isNotEmpty()) { nav.back() }
    BackHandler(nav.stage) { nav.stage = false }
    val clock = rememberPlayClock(s)
    val hasTrack = s.track != null
    val now = NowMark(s.track?.id, !s.paused)
    CompositionLocalProvider(LocalNav provides nav, LocalNow provides now) {
    SharedTransitionLayout(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            Box(Modifier.weight(1f)) {
                // Вкладки листаются свайпом вбок.
                // Вкладка переключается уже с пятой части ширины, а не с половины.
                HorizontalPager(
                    pager,
                    key = { it },
                    flingBehavior = PagerDefaults.flingBehavior(pager, snapPositionalThreshold = 0.2f),
                ) { page ->
                    when (Tab.entries[page]) {
                        Tab.Wave -> WaveTab(app, pal, s)
                        Tab.Library -> LibraryTab(app, nav, s.likesLoading)
                        Tab.Search -> SearchTab(app, nav)
                        Tab.Releases -> ReleasesTab(app, nav)
                        Tab.Settings -> SettingsTab(app)
                    }
                }
                PageHost(app, nav)
                androidx.compose.animation.AnimatedVisibility(
                    !nav.stage && hasTrack,
                    Modifier.align(Alignment.BottomCenter),
                    enter = fadeIn(),
                    exit = fadeOut(),
                ) {
                    MiniPlayer(app, s, clock, this@SharedTransitionLayout, this) { nav.stage = true }
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
        AnimatedVisibility(nav.stage && hasTrack, enter = fadeIn(), exit = fadeOut()) {
            Stage(app, pal, s, clock, this@SharedTransitionLayout, this) { nav.stage = false }
        }
    }
    QueueSheet(app, nav, s.queuePos?.toInt())
    TrackMenu(app, nav)
    }
}

// ── Моя волна ────────────────────────────────────────────────────────────────

@Composable
private fun WaveTab(app: YaApp, pal: Pal, s: PlayerState) {
    val presets by app.wheel.collectAsStateWithLifecycle()
    val waveOn = s.wave
    val playing = waveOn && s.track != null && !s.paused
    Box(Modifier.fillMaxSize()) {
        Blob(pal.blob, playing, Modifier.fillMaxSize())
        Box(
            Modifier.fillMaxSize().background(
                Brush.verticalGradient(0.55f to Color.Transparent, 1f to MaterialTheme.colorScheme.background),
            ),
        )
        Column(Modifier.fillMaxSize().statusBarsPadding().padding(top = 24.dp, bottom = 120.dp)) {
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
    Box(Modifier.fillMaxSize()) {
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 120.dp)) {
        itemsIndexed(shown, key = { _, it -> it.index }, contentType = { _, _ -> "track" }) { _, (i, t) ->
            TrackRow(
                app, t, current = now.id == t.id, playing = now.playing,
                onLongClick = { nav.menu = MenuTarget(t, uniffi.yamusic_android.Source.Likes, i) },
            ) { app.core.playLikes(i.toUInt()) }
        }
    }
    FloatingPlay { app.core.playLikes(0u) }
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
fun SearchPill(value: String, onChange: (String) -> Unit, hint: String, onSubmit: (() -> Unit)? = null) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp).clip(RoundedCornerShape(28.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh).padding(horizontal = 18.dp, vertical = 14.dp),
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
                keyboardActions = KeyboardActions(onSearch = { onSubmit?.invoke() }),
                modifier = Modifier.fillMaxWidth(),
            )
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
    open: () -> Unit,
) {
    val t = s.track ?: return
    val scope = rememberCoroutineScope()
    val dx = remember { Animatable(0f) }
    val density = LocalDensity.current
    val skip = with(density) { 56.dp.toPx() }
    val up = with(density) { 28.dp.toPx() }
    with(shared) {
        Surface(
            shape = RoundedCornerShape(32.dp),
            color = MaterialTheme.colorScheme.surfaceContainerHigh,
            tonalElevation = 6.dp,
            shadowElevation = 12.dp,
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 10.dp)
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
                                    x < -skip -> {
                                        app.core.next()
                                        dx.animateTo(-size.width.toFloat(), tween(160))
                                        dx.snapTo(size.width.toFloat() * 0.4f)
                                        dx.animateTo(0f, spring(dampingRatio = 0.8f))
                                    }
                                    x > skip -> {
                                        app.core.prev()
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
                        size = 44.dp,
                        container = Color.Transparent,
                        tint = if (s.liked) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    PlayButton(!s.paused, 52.dp, onClick = { app.core.playPause() })
                    ShapeIconButton(Ic.Next, "Дальше", { app.core.next() }, size = 44.dp, container = Color.Transparent)
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
    close: () -> Unit,
) {
    val t = s.track ?: return
    var showLyrics by rememberSaveable { mutableStateOf(false) }
    val lyr by app.lyrics.collectAsStateWithLifecycle()
    val lyrics = lyr?.takeIf { it.trackId == t.id }?.lyrics
    val scope = rememberCoroutineScope()
    val dy = remember { Animatable(0f) }
    val density = LocalDensity.current
    val dismiss = with(density) { 80.dp.toPx() }
    val lyricsOn = showLyrics && lyrics != null
    val scrim by animateFloatAsState(if (lyricsOn) 0.62f else 0f, tween(400), label = "scrim")
    // Чем дальше утянули, тем сильнее скругление и меньше масштаб — видно, что сворачивается.
    val pull = (dy.value / with(density) { 400.dp.toPx() }).coerceIn(0f, 1f)
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
                    val k = 1f - pull * 0.08f
                    scaleX = k
                    scaleY = k
                    shape = RoundedCornerShape((pull * 40).dp)
                    clip = pull > 0f
                }
                .background(MaterialTheme.colorScheme.background)
                .pointerInput(Unit) {
                    detectVerticalDragGestures(
                        onDragEnd = {
                            scope.launch {
                                if (dy.value > dismiss) close() else dy.animateTo(0f, spring(dampingRatio = 0.75f))
                            }
                        },
                        onDragCancel = { scope.launch { dy.animateTo(0f) } },
                    ) { change, d ->
                        change.consume()
                        scope.launch { dy.snapTo((dy.value + d).coerceAtLeast(0f)) }
                    }
                },
        ) {
            Blob(pal.blob, !s.paused, Modifier.fillMaxSize())
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
                    ShapeIconButton(
                        Ic.Lyrics,
                        "Текст песни",
                        { showLyrics = !showLyrics },
                        size = 48.dp,
                        container = if (showLyrics) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceContainerHighest.copy(alpha = 0.7f),
                        tint = if (showLyrics) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface,
                    )
                    Spacer(Modifier.width(8.dp))
                    val nav = LocalNav.current
                    ShapeIconButton(Ic.More, "Ещё", {
                        val i = s.queuePos?.toInt()
                        if (i != null) nav.menu = MenuTarget(t, uniffi.yamusic_android.Source.Queue, i)
                    }, size = 48.dp)
                }
                BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
                    val wide = maxWidth > maxHeight * 1.2f
                    val art: @Composable (Modifier) -> Unit = { m ->
                        BoxWithConstraints(m, contentAlignment = Alignment.Center) {
                            if (lyricsOn && lyrics != null) {
                                LyricsView(app, lyrics, clock)
                            } else {
                                CoverCarousel(app, s, t, maxWidth, maxHeight, shared, visibility)
                            }
                        }
                    }
                    if (wide) {
                        Row(Modifier.fillMaxSize().padding(vertical = 12.dp), horizontalArrangement = Arrangement.spacedBy(32.dp)) {
                            art(Modifier.weight(1f).fillMaxHeight())
                            Column(Modifier.weight(1f).fillMaxHeight(), verticalArrangement = Arrangement.Bottom) {
                                StageInfo(app, s, t, clock)
                            }
                        }
                    } else {
                        // Кнопки — в зоне большого пальца: под ними остаётся заметный запас.
                        Column(Modifier.fillMaxSize()) {
                            art(Modifier.weight(1f).fillMaxWidth().padding(vertical = 16.dp))
                            StageInfo(app, s, t, clock)
                            Spacer(Modifier.height(24.dp))
                        }
                    }
                }
            }
        }
    }
}

/**
 * Обложки очереди каруселью: соседние выглядывают по краям, свайп листает и включает трек.
 * Если очередь ещё не пришла или не совпадает с играющим — одна обложка со свайпом.
 */
@OptIn(ExperimentalSharedTransitionApi::class)
@Composable
private fun CoverCarousel(
    app: YaApp,
    s: PlayerState,
    t: TrackItem,
    width: Dp,
    height: Dp,
    shared: SharedTransitionScope,
    visibility: AnimatedVisibilityScope,
) {
    val q by app.queue.collectAsStateWithLifecycle()
    val items = q.second
    val pos = s.queuePos?.toInt()
    // Обложка поуже экрана, чтобы соседние были видны.
    val side = minOf(width * 0.8f, height, 560.dp)
    if (pos == null || pos !in items.indices || items[pos].id != t.id) {
        SwipeCover(app, t, side, shared, visibility)
        return
    }
    val pager = rememberPagerState(initialPage = pos) { items.size }
    val current by rememberUpdatedState(pos)
    var auto by remember { mutableStateOf(false) }
    // Трек сменился не свайпом (кнопка, конец трека) — карусель догоняет.
    LaunchedEffect(pos, items.size) {
        if (pager.currentPage != pos) {
            auto = true
            try {
                if (kotlin.math.abs(pager.currentPage - pos) > 3) pager.scrollToPage(pos) else pager.animateScrollToPage(pos)
            } finally {
                auto = false
            }
        }
    }
    LaunchedEffect(pager) {
        snapshotFlow { pager.settledPage }.collect { p ->
            if (!auto && p != current && p in items.indices) app.core.playFrom(uniffi.yamusic_android.Source.Queue, p.toUInt())
        }
    }
    // Карусель шире колонки сцены (её отступы по 24) — соседние обложки доходят до края экрана.
    val full = width + 48.dp
    HorizontalPager(
        pager,
        Modifier.requiredWidth(full),
        contentPadding = PaddingValues(horizontal = (full - side) / 2),
        pageSpacing = 12.dp,
        beyondViewportPageCount = 1,
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
            if (page == pos) {
                with(shared) {
                    Art(app, it.cover, Modifier.sharedElement(rememberSharedContentState(KEY_COVER), visibility).size(side), corner = 36.dp)
                }
            } else {
                Art(app, it.cover, Modifier.size(side), corner = 36.dp)
            }
        }
    }
}

/** Обложка сцены: свайп вбок — соседний трек (улетает и возвращается уже новой). */
@OptIn(ExperimentalSharedTransitionApi::class)
@Composable
private fun SwipeCover(app: YaApp, t: TrackItem, side: Dp, shared: SharedTransitionScope, visibility: AnimatedVisibilityScope) {
    val scope = rememberCoroutineScope()
    val dx = remember { Animatable(0f) }
    val skip = with(LocalDensity.current) { 56.dp.toPx() }
    with(shared) {
        Art(
            app,
            t.cover,
            Modifier
                .sharedElement(rememberSharedContentState(KEY_COVER), visibility)
                .size(side)
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
                                        app.core.prev()
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
private fun ColumnScope.StageInfo(app: YaApp, s: PlayerState, t: TrackItem, clock: PlayClock) {
    // Длинное название бежит строкой, как marquee в TUI.
    ScrambleText(
        t.title,
        MaterialTheme.typography.headlineMedium.copy(fontWeight = FontWeight.Black),
        MaterialTheme.colorScheme.onSurface,
        Modifier.basicMarquee(initialDelayMillis = 2000),
    )
    val nav = LocalNav.current
    // Исполнитель и альбом — ссылки на свои страницы.
    Text(t.artists, style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSurface,
        maxLines = 1, overflow = TextOverflow.Ellipsis,
        modifier = Modifier.clip(RoundedCornerShape(10.dp)).clickable(enabled = t.artistId != null) { nav.artist(t.artistId, t.artists) })
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
    // Очередь — пилюля под кнопками, в зоне большого пальца.
    Row(
        Modifier.align(Alignment.CenterHorizontally).clip(RoundedCornerShape(24.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHighest.copy(alpha = 0.7f))
            .clickable { nav.queue = true }.padding(horizontal = 20.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Ic.Queue, null, Modifier.size(20.dp))
        Spacer(Modifier.width(10.dp))
        Text("Очередь", style = MaterialTheme.typography.labelLarge)
        if (s.queueLen > 0u) {
            Spacer(Modifier.width(10.dp))
            Text("${(s.queuePos?.toInt() ?: 0) + 1} из ${s.queueLen}", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary)
        }
    }
    Spacer(Modifier.height(8.dp))
}

/**
 * Текст песни: синхронный — текущая строка крупно и в акценте (с тенью, чтобы читалась над
 * блобом), держится на трети высоты; тап по строке — перемотка. Строка пересчитывается из
 * часов позиции, а перекомпоновка случается только при смене строки.
 */
@Composable
private fun LyricsView(app: YaApp, l: LyricsData, clock: PlayClock) {
    if (!l.synced) {
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
            l.lines.forEach { Text(it.text, style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(vertical = 4.dp)) }
        }
        return
    }
    val cur by remember(l) { derivedStateOf { l.lines.indexOfLast { it.timeMs.toLong() <= clock.pos() + 250 } } }
    val list = rememberLazyListState()
    LaunchedEffect(cur) { if (cur >= 0) list.animateScrollToItem(cur) }
    val glow = Shadow(Color.Black.copy(alpha = 0.75f), blurRadius = 18f)
    BoxWithConstraints(Modifier.fillMaxSize()) {
        LazyColumn(Modifier.fillMaxSize(), state = list, contentPadding = PaddingValues(top = maxHeight / 3, bottom = maxHeight * 2 / 3)) {
            itemsIndexed(l.lines) { i, line ->
                val active = i == cur
                val color by animateColorAsState(
                    when {
                        active -> MaterialTheme.colorScheme.primary
                        i < cur -> MaterialTheme.colorScheme.onSurface.copy(alpha = 0.4f)
                        else -> MaterialTheme.colorScheme.onSurface.copy(alpha = 0.7f)
                    },
                    label = "line",
                )
                val style = if (active) MaterialTheme.typography.headlineSmall else MaterialTheme.typography.titleLarge
                Text(
                    line.text.ifBlank { "♪" },
                    style = style.copy(shadow = glow),
                    fontWeight = if (active) FontWeight.Black else FontWeight.Bold,
                    color = color,
                    modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp))
                        .clickable { app.core.seek(line.timeMs) }.padding(vertical = 8.dp, horizontal = 4.dp),
                )
            }
        }
    }
}
