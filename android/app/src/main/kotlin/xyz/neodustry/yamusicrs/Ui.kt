@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package xyz.neodustry.yamusicrs

import androidx.activity.compose.BackHandler
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

private val Ink = Color(0xFF0B0B0C)

// ── тема ─────────────────────────────────────────────────────────────────────

/** Схема MD3 из палитры обложки (как тема TUI): акценты, тонированные поверхности, текст. */
private fun scheme(p: Pal): ColorScheme {
    val bg = lerp(Ink, p.surface, 0.35f)
    return darkColorScheme(
        primary = p.accent,
        onPrimary = Ink,
        primaryContainer = lerp(bg, p.accent, 0.32f),
        onPrimaryContainer = Color.White,
        secondary = p.accent2,
        onSecondary = Ink,
        secondaryContainer = lerp(bg, p.accent2, 0.28f),
        onSecondaryContainer = Color.White,
        tertiary = p.blob[2],
        background = bg,
        onBackground = lerp(Color(0xFFE8E8E8), p.accent, 0.08f),
        surface = bg,
        onSurface = lerp(Color(0xFFE8E8E8), p.accent, 0.08f),
        onSurfaceVariant = lerp(Color(0xFF8C8C8C), p.accent, 0.15f),
        surfaceContainerLowest = lerp(bg, Ink, 0.5f),
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
    MaterialExpressiveTheme(colorScheme = s, motionScheme = MotionScheme.expressive(), content = content)
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

private enum class Tab(val title: String) { Wave("Волна"), Library("Коллекция"), Queue("Очередь") }

@Composable
private fun Main(app: YaApp, pal: Pal, s: PlayerState) {
    var tab by rememberSaveable { mutableIntStateOf(0) }
    var stage by rememberSaveable { mutableStateOf(false) }
    BackHandler(stage) { stage = false }
    val motion = MaterialTheme.motionScheme
    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            Box(Modifier.weight(1f)) {
                AnimatedContent(
                    Tab.entries[tab],
                    transitionSpec = {
                        (fadeIn(motion.defaultEffectsSpec()) + scaleIn(initialScale = 0.96f))
                            .togetherWith(fadeOut(motion.fastEffectsSpec()))
                    },
                    label = "tab",
                ) { t ->
                    when (t) {
                        Tab.Wave -> WaveTab(app, pal, s)
                        Tab.Library -> LibraryTab(app, s)
                        Tab.Queue -> QueueTab(app, s)
                    }
                }
                androidx.compose.animation.AnimatedVisibility(
                    s.track != null,
                    Modifier.align(Alignment.BottomCenter),
                    enter = slideInVertically { it } + fadeIn(),
                    exit = slideOutVertically { it } + fadeOut(),
                ) {
                    MiniPlayer(app, s) { stage = true }
                }
            }
            ShortNavigationBar(containerColor = MaterialTheme.colorScheme.surfaceContainerLow) {
                Tab.entries.forEachIndexed { i, t ->
                    ShortNavigationBarItem(
                        selected = tab == i,
                        onClick = { tab = i },
                        icon = {
                            Icon(
                                when (t) {
                                    Tab.Wave -> Ic.Wave
                                    Tab.Library -> Ic.Library
                                    Tab.Queue -> Ic.Queue
                                },
                                null,
                            )
                        },
                        label = { Text(t.title) },
                    )
                }
            }
        }
        AnimatedVisibility(
            stage && s.track != null,
            enter = slideInVertically(MaterialTheme.motionScheme.defaultSpatialSpec()) { it } + fadeIn(),
            exit = slideOutVertically(MaterialTheme.motionScheme.fastSpatialSpec()) { it } + fadeOut(),
        ) {
            Stage(app, pal, s) { stage = false }
        }
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
                val sub = when {
                    s.waveLoading -> "настраиваю…"
                    waveOn && s.track != null -> "${s.track!!.title} — ${s.track!!.artists}"
                    s.offline -> "без сети — по скачанному"
                    else -> "бесконечный поток под ваш вкус"
                }
                Text(sub, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                if (s.waveLoading) {
                    LoadingIndicator(Modifier.size(120.dp))
                } else {
                    PlayButton(playing, 132.dp, onClick = {
                        if (waveOn && s.track != null) app.core.playPause() else app.core.startWave()
                    })
                }
            }
            if (presets.isNotEmpty()) {
                LazyRow(
                    contentPadding = PaddingValues(horizontal = 20.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    items(presets, key = { it.seeds.joinToString() }) { p ->
                        PresetCard(app, p, active = waveOn && s.waveSeeds == p.seeds) {
                            app.core.startWaveWith(p.seeds, p.name)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun PresetCard(app: YaApp, p: WavePreset, active: Boolean, onClick: () -> Unit) {
    val tint = p.color?.let { Color(0xFF000000 or it.toLong()) } ?: MaterialTheme.colorScheme.primary
    val bg by animateColorAsState(
        if (active) lerp(MaterialTheme.colorScheme.surfaceContainerHigh, tint, 0.45f)
        else MaterialTheme.colorScheme.surfaceContainer.copy(alpha = 0.82f),
        label = "preset",
    )
    val shape = if (active) MaterialShapes.Cookie12Sided.toShape() else MaterialShapes.Circle.toShape()
    Column(
        Modifier.width(132.dp).clip(RoundedCornerShape(28.dp)).background(bg).clickable(onClick = onClick).padding(14.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(Modifier.size(64.dp).clip(shape).background(tint.copy(alpha = 0.35f))) {
            Art(app, p.icon, Modifier.fillMaxSize(), corner = 0.dp)
        }
        Spacer(Modifier.height(10.dp))
        Text(p.name, style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
        p.description?.let {
            Text(it, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2, overflow = TextOverflow.Ellipsis, textAlign = TextAlign.Center)
        }
    }
}

// ── коллекция и очередь ──────────────────────────────────────────────────────

@Composable
private fun LibraryTab(app: YaApp, s: PlayerState) {
    val likes by app.likes.collectAsStateWithLifecycle()
    var filter by rememberSaveable { mutableStateOf("") }
    val shown = remember(likes, filter) {
        val q = filter.trim().lowercase()
        likes.withIndex().filter { (_, t) -> q.isEmpty() || t.title.lowercase().contains(q) || t.artists.lowercase().contains(q) }
    }
    Column(Modifier.fillMaxSize().statusBarsPadding()) {
        Header("Мне нравится", if (likes.isEmpty()) "" else "${likes.size} треков") {
            if (likes.isNotEmpty()) PlayButton(false, 56.dp, onClick = { app.core.playLikes(0u) })
        }
        SearchPill(filter, { filter = it }, "Поиск по коллекции")
        if (likes.isEmpty()) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                if (s.likesLoading) LoadingIndicator() else Text("Пусто")
            }
            return@Column
        }
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 120.dp)) {
            itemsIndexed(shown, key = { _, it -> it.index }) { _, (i, t) ->
                TrackRow(app, t, current = s.track?.id == t.id, playing = !s.paused) { app.core.playLikes(i.toUInt()) }
            }
        }
    }
}

@Composable
private fun QueueTab(app: YaApp, s: PlayerState) {
    val q by app.queue.collectAsStateWithLifecycle()
    val (title, items) = q
    val list = rememberLazyListState()
    LaunchedEffect(title, items.size) { s.queuePos?.let { list.scrollToItem((it.toInt() - 2).coerceAtLeast(0)) } }
    Column(Modifier.fillMaxSize().statusBarsPadding()) {
        Header("Очередь", if (items.isEmpty()) "" else "$title · ${items.size}") {}
        if (items.isEmpty()) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text("Очередь пуста", color = MaterialTheme.colorScheme.onSurfaceVariant) }
            return@Column
        }
        LazyColumn(Modifier.fillMaxSize(), state = list, contentPadding = PaddingValues(bottom = 120.dp)) {
            itemsIndexed(items) { i, t ->
                val cur = s.queuePos?.toInt() == i
                TrackRow(app, t, current = cur, playing = !s.paused, dim = s.queuePos?.let { i < it.toInt() } == true) {
                    app.core.playQueue(i.toUInt())
                }
            }
        }
    }
}

@Composable
private fun Header(title: String, subtitle: String, action: @Composable () -> Unit) {
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
private fun SearchPill(value: String, onChange: (String) -> Unit, hint: String) {
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
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}

@Composable
private fun TrackRow(app: YaApp, t: TrackItem, current: Boolean, playing: Boolean, dim: Boolean = false, onClick: () -> Unit) {
    val alpha = when {
        !t.available -> 0.35f
        dim -> 0.55f
        else -> 1f
    }
    val bg by animateColorAsState(
        if (current) MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.55f) else Color.Transparent,
        label = "row",
    )
    Row(
        // Концентрично миниатюре: 14 + 10 (отступ) = 24.
        Modifier.fillMaxWidth().padding(horizontal = 8.dp).clip(RoundedCornerShape(24.dp)).background(bg)
            .clickable(enabled = t.available, onClick = onClick).padding(horizontal = 10.dp, vertical = 8.dp),
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

@Composable
private fun MiniPlayer(app: YaApp, s: PlayerState, open: () -> Unit) {
    val t = s.track ?: return
    val pos = smoothPosition(s)
    val frac = (pos.toFloat() / t.durationMs.toLong().coerceAtLeast(1)).coerceIn(0f, 1f)
    Surface(
        shape = RoundedCornerShape(32.dp),
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        tonalElevation = 6.dp,
        shadowElevation = 12.dp,
        modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 10.dp).clickable(onClick = open),
    ) {
        Box {
            // Прогресс — заливкой фона пилюли.
            val fill = MaterialTheme.colorScheme.primary.copy(alpha = 0.14f)
            Box(Modifier.matchParentSize().drawBehind { drawRect(fill, size = Size(size.width * frac, size.height)) })
            Row(Modifier.padding(8.dp), verticalAlignment = Alignment.CenterVertically) {
                // Концентрично пилюле: 32 − 8 (отступ) = 24.
                Art(app, t.thumb, Modifier.size(52.dp), corner = 24.dp)
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    ScrambleText(
                        t.title,
                        MaterialTheme.typography.titleMedium.copy(fontWeight = FontWeight.Bold),
                        MaterialTheme.colorScheme.onSurface,
                        Modifier.basicMarquee(initialDelayMillis = 2000),
                    )
                    Text(
                        if (s.loading) "загрузка…" else t.artists,
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

// ── сцена ────────────────────────────────────────────────────────────────────

@Composable
private fun Stage(app: YaApp, pal: Pal, s: PlayerState, close: () -> Unit) {
    val t = s.track ?: return
    var showLyrics by rememberSaveable { mutableStateOf(false) }
    val lyr by app.lyrics.collectAsStateWithLifecycle()
    val lyrics = lyr?.takeIf { it.trackId == t.id }?.lyrics
    val pos = smoothPosition(s)
    Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).clickable(enabled = false) {}) {
        Blob(pal.blob, !s.paused, Modifier.fillMaxSize())
        Box(
            Modifier.fillMaxSize().background(
                Brush.verticalGradient(0f to Color.Transparent, 0.5f to Color.Transparent, 1f to MaterialTheme.colorScheme.background.copy(alpha = 0.92f)),
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
                    "Текст",
                    { showLyrics = !showLyrics },
                    size = 48.dp,
                    container = if (showLyrics) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceContainerHighest.copy(alpha = 0.7f),
                    tint = if (showLyrics) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface,
                )
            }
            BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
                val wide = maxWidth > maxHeight * 1.2f
                val art: @Composable (Modifier) -> Unit = { m ->
                    BoxWithConstraints(m, contentAlignment = Alignment.Center) {
                        AnimatedContent(showLyrics && lyrics != null, label = "lyrics") { lyricsOn ->
                            if (lyricsOn && lyrics != null) {
                                LyricsView(app, lyrics, pos)
                            } else {
                                val side = minOf(maxWidth, maxHeight, 560.dp)
                                Art(app, t.cover, Modifier.size(side), corner = 36.dp)
                            }
                        }
                    }
                }
                val info: @Composable (Modifier) -> Unit = { m ->
                    Column(m, verticalArrangement = Arrangement.Bottom) {
                        StageInfo(app, s, t, pos)
                    }
                }
                if (wide) {
                    Row(Modifier.fillMaxSize().padding(vertical = 12.dp), horizontalArrangement = Arrangement.spacedBy(32.dp)) {
                        art(Modifier.weight(1f).fillMaxHeight())
                        info(Modifier.weight(1f).fillMaxHeight())
                    }
                } else {
                    Column(Modifier.fillMaxSize()) {
                        art(Modifier.weight(1f).fillMaxWidth().padding(vertical = 16.dp))
                        info(Modifier.fillMaxWidth())
                    }
                }
            }
        }
    }
}

/** Название, исполнитель, перемотка и кнопки сцены. */
@Composable
private fun StageInfo(app: YaApp, s: PlayerState, t: TrackItem, pos: Long) {
    // Длинное название бежит строкой, как marquee в TUI.
    ScrambleText(
        t.title,
        MaterialTheme.typography.headlineMedium.copy(fontWeight = FontWeight.Black),
        MaterialTheme.colorScheme.onSurface,
        Modifier.basicMarquee(initialDelayMillis = 2000),
    )
    Text(
        listOf(t.artists, t.album).filter { it.isNotEmpty() }.joinToString(" · "),
        style = MaterialTheme.typography.titleMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
    )
    Spacer(Modifier.height(12.dp))
    WavySeek(pos, t.durationMs.toLong(), !s.paused && !s.loading, { app.core.seek(it.toULong()) })
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
    Spacer(Modifier.height(8.dp))
}

/** Текст песни: синхронный — текущая строка крупно и в акценте, список сам едет к ней (как в TUI). */
@Composable
private fun LyricsView(app: YaApp, l: LyricsData, posMs: Long) {
    if (!l.synced) {
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
            l.lines.forEach { Text(it.text, style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(vertical = 4.dp)) }
        }
        return
    }
    val cur = l.lines.indexOfLast { it.timeMs.toLong() <= posMs + 250 }
    val list = rememberLazyListState()
    // Текущая строка держится на трети высоты; запас снизу — чтобы и последние до неё доехали.
    LaunchedEffect(cur) {
        if (cur >= 0) list.animateScrollToItem(cur)
    }
    BoxWithConstraints(Modifier.fillMaxSize()) {
    LazyColumn(Modifier.fillMaxSize(), state = list, contentPadding = PaddingValues(top = maxHeight / 3, bottom = maxHeight * 2 / 3)) {
        itemsIndexed(l.lines) { i, line ->
            val active = i == cur
            val color by animateColorAsState(
                when {
                    active -> MaterialTheme.colorScheme.primary
                    i < cur -> MaterialTheme.colorScheme.onSurface.copy(alpha = 0.35f)
                    else -> MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f)
                },
                label = "line",
            )
            Text(
                line.text.ifBlank { "♪" },
                style = if (active) MaterialTheme.typography.headlineSmall else MaterialTheme.typography.titleLarge,
                fontWeight = if (active) FontWeight.Black else FontWeight.Bold,
                color = color,
                modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp))
                    .clickable { app.core.seek(line.timeMs) }.padding(vertical = 8.dp, horizontal = 4.dp),
            )
        }
    }
    }
}
