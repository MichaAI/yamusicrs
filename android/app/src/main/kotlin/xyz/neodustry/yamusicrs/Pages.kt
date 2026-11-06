@file:OptIn(ExperimentalMaterial3ExpressiveApi::class, ExperimentalMaterial3Api::class)

package xyz.neodustry.yamusicrs

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.SizeTransform
import androidx.compose.animation.core.SeekableTransitionState
import androidx.compose.animation.core.animateDp
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.Canvas
import androidx.compose.animation.core.rememberTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.scaleOut
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.findRootCoordinates
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyGridScope
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import kotlinx.coroutines.launch
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.draw.drawBehind
import androidx.compose.material3.ButtonGroupDefaults
import androidx.compose.material3.ToggleButton
import androidx.compose.material3.ToggleButtonDefaults
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.ui.composed
import androidx.compose.ui.layout.layout
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlin.math.roundToInt
import uniffi.yamusic_android.AlbumItem
import uniffi.yamusic_android.ArtistItem
import uniffi.yamusic_android.Page
import uniffi.yamusic_android.PlaylistItem
import uniffi.yamusic_android.SettingsData
import uniffi.yamusic_android.Source
import uniffi.yamusic_android.TrackItem

// ── навигация ────────────────────────────────────────────────────────────────

/** Открытая страница: id запроса в ядре и заголовок, пока она грузится. */
data class Route(
    val id: ULong,
    val title: String,
    /** Порядковый номер открытия: ключ перехода и его направление (id ядра у «полного списка» общий с родителем). */
    val seq: Int = 0,
    /** «Весь список» вкладки исполнителя: страница ядра — `id`, вкладка — `tab`. */
    val tab: Int = -1,
    /** Адрес для «Поделиться» (без параметров), если у страницы он есть. */
    val share: String? = null,
    /** Обложка нажатой карточки и вкладка, на которой она была: для полёта обратно. */
    val tap: Tap? = null,
    val origin: Int = -1,
)

private const val MUSIC_HOST = "https://music.yandex.ru"

/** Ссылка в том виде, как её отдаёт «Поделиться» официального клиента. */
fun shareLink(path: String): String =
    "$MUSIC_HOST$path?utm_medium=copy_link&ref_id=${java.util.UUID.randomUUID()}"

fun shareText(context: android.content.Context, path: String) {
    val send = android.content.Intent(android.content.Intent.ACTION_SEND)
        .setType("text/plain")
        .putExtra(android.content.Intent.EXTRA_TEXT, shareLink(path))
    context.startActivity(android.content.Intent.createChooser(send, null).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK))
}

/**
 * Запоминает рамку обложки карточки; `tap()` перед переходом отдаёт её навигации, и страница
 * принимает обложку на своё место.
 */
@Composable
fun rememberCoverTap(url: String?): Pair<Modifier, () -> Unit> {
    val nav = LocalNav.current
    // Рамку считаем только по нажатию: при прокрутке колбэк лишь запоминает координаты.
    val box = remember { arrayOfNulls<androidx.compose.ui.layout.LayoutCoordinates>(1) }
    val mod = Modifier.onGloballyPositioned { box[0] = it }
    val tap = {
        box[0]?.takeIf { it.isAttached }?.let { c -> if (url != null) nav.tapped = Tap(c.boundsInRoot(), url, 20.dp) }
        Unit
    }
    return mod to tap
}

/** «Скачано N из M» с полосой: сколько списка уже играет без сети. */
@Composable
private fun DownloadProgress(have: Int, total: Int) {
    val f by animateFloatAsState(have.toFloat() / total, MaterialTheme.motionScheme.defaultSpatialSpec(), label = "have")
    Column(Modifier.fillMaxWidth(0.7f), horizontalAlignment = Alignment.CenterHorizontally) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(Ic.Download, null, Modifier.size(16.dp), tint = MaterialTheme.colorScheme.primary)
            Spacer(Modifier.width(6.dp))
            Text(
                if (have == total) "Скачан целиком" else "Скачано $have из $total",
                style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary,
            )
        }
        Spacer(Modifier.height(6.dp))
        Box(Modifier.fillMaxWidth().height(4.dp).clip(RoundedCornerShape(2.dp)).background(MaterialTheme.colorScheme.surfaceContainerHighest)) {
            Box(Modifier.fillMaxWidth(f).fillMaxHeight().background(MaterialTheme.colorScheme.primary))
        }
    }
}

/** Откуда летит обложка: рамка в окне, адрес картинки, скругление. */
data class Tap(val from: androidx.compose.ui.geometry.Rect, val url: String, val corner: Dp)

/** Полёт обложки на страницу `routeId`. */
data class Fly(val routeId: ULong, val tap: Tap)

/**
 * Обратный полёт: обложка с шапки закрываемой страницы возвращается на свою карточку.
 * `t` — 0..1; жест «назад» ведёт его за пальцем (`manual`), иначе он доезжает сам.
 */
class FlyBack(val routeId: ULong, val tap: Tap, val from: androidx.compose.ui.geometry.Rect) {
    var t by androidx.compose.runtime.mutableFloatStateOf(0f)
    var manual = false
}

/** Трек, для которого открыто меню: откуда он и под каким индексом. */
data class MenuTarget(val track: TrackItem, val source: Source, val index: Int)

/**
 * Стек страниц поверх вкладок, сцена, очередь и меню трека. Переход на страницу
 * сворачивает сцену и очередь — страница открывается под мини-плеером.
 */
class Nav(private val app: YaApp) {
    val stack = mutableStateListOf<Route>()
    var stage by mutableStateOf(false)
    var queue by mutableStateOf(false)
    var menu by mutableStateOf<MenuTarget?>(null)
    var devices by mutableStateOf(false)
    /** Выбор исполнителя, когда у трека их несколько. */
    var artists by mutableStateOf<List<ArtistItem>?>(null)
    /** Текст песни листом, чтобы выделить и скопировать. */
    var lyrics by mutableStateOf(false)

    /** Обложка карточки, по которой только что нажали: страница, что откроется, «посадит» её на своё место. */
    var tapped: Tap? = null
    var fly by mutableStateOf<Fly?>(null)
    var flyTarget by mutableStateOf<androidx.compose.ui.geometry.Rect?>(null)
    var flyUrl: String? = null

    /** Текущая вкладка приложения (задаёт Main): обратный полёт только на ту же вкладку. */
    var tabNow: () -> Int = { 0 }
    /** Где сейчас обложка-шапка каждой открытой страницы; пишется без рекомпозиции. */
    val heroRects = HashMap<ULong, androidx.compose.ui.geometry.Rect>()
    var flyBack by mutableStateOf<FlyBack?>(null)

    private var seq = 0

    private fun push(id: ULong, title: String, tab: Int = -1, share: String? = null) {
        val tap = tapped
        fly = tap?.let { Fly(id, it) }
        flyTarget = null
        tapped = null
        stage = false
        queue = false
        menu = null
        artists = null
        lyrics = false
        stack.add(Route(id, title, ++seq, tab, share, tap, tabNow()))
    }

    /** К исполнителю трека; если их несколько — сначала выбрать, к какому. */
    fun artistOf(t: TrackItem) {
        val list = t.artistList
        when {
            list.size > 1 -> {
                menu = null
                artists = list
            }
            list.size == 1 -> artist(list[0].id, list[0].name)
            else -> artist(t.artistId, t.artists)
        }
    }

    fun artist(id: String?, name: String) {
        push(app.core.openArtist(id ?: return), name, share = "/artist/$id")
    }

    /** Весь список вкладки исполнителя — отдельным экраном (страница ядра та же). */
    fun artistTracks(pageId: ULong, tab: Int, title: String) = push(pageId, title, tab)

    fun album(id: String?, title: String) {
        push(app.core.openAlbum(id ?: return), title, share = "/album/$id")
    }

    fun playlist(p: PlaylistItem) = push(app.core.openPlaylist(p.uid, p.kind), p.title, share = "/users/${p.uid}/playlists/${p.kind}")

    fun similar(t: TrackItem) = push(app.core.openSimilar(t.id, t.title), "Похожие")

    fun chart() = push(app.core.chart(), "Чарт")

    /**
     * Обложка вернётся на карточку, только если страница одна над вкладкой, с которой её открыли,
     * и шапка сейчас на виду. Иначе остаётся обычный уход страницы.
     */
    fun prepareFlyBack(r: Route): FlyBack? {
        val tap = r.tap ?: return null
        if (stack.size != 1 || r.origin != tabNow()) return null
        val h = heroRects[r.id] ?: return null
        if (h.top < 0f || h.width <= 0f) return null
        return FlyBack(r.id, tap, h).also { flyBack = it }
    }

    fun back(animated: Boolean = true) {
        fly = null
        val top = stack.lastOrNull()
        if (animated && top != null && flyBack == null) prepareFlyBack(top)
        stack.removeLastOrNull()?.let {
            heroRects.remove(it.id)
            if (it.tab < 0) app.closePage(it.id)
        }
    }

    fun clear() {
        flyBack = null
        while (stack.isNotEmpty()) back(animated = false)
    }
}

val LocalNav = staticCompositionLocalOf<Nav> { error("Nav не задан") }

/** Играющее сейчас: id трека и идёт ли звук (для подсветки строк). */
data class NowMark(val id: String?, val playing: Boolean)

val LocalNow = compositionLocalOf { NowMark(null, false) }

// ── хост страниц ─────────────────────────────────────────────────────────────

/**
 * Верх стека — поверх вкладок; въезжает справа, уезжает вправо. Жест «назад» предиктивный:
 * переход к странице под ней идёт за пальцем — верхняя уменьшается, скругляется и отъезжает,
 * нижняя (или вкладки) проступает. Отпустили — доезжает, передумали — возвращается.
 */
@Composable
fun PageHost(app: YaApp, nav: Nav) {
    val top = nav.stack.lastOrNull()
    val state = remember { SeekableTransitionState(top) }
    val pageTransition = rememberTransition(state, label = "page")
    // С какого края тянут: от правого страница уезжает влево.
    var edge by remember { mutableIntStateOf(androidx.activity.BackEventCompat.EDGE_LEFT) }
    // Открыли, закрыли кнопкой, сменили вкладку — обычный переход.
    LaunchedEffect(top) {
        if (state.currentState != top || state.targetState != top) state.animateTo(top)
    }
    WindowBackHandler(enabled = nav.stack.isNotEmpty() && !nav.stage) { progress ->
        val below = nav.stack.getOrNull(nav.stack.size - 2)
        // Обложка шапки летит на свою карточку за пальцем.
        val fb = nav.stack.lastOrNull()?.let { nav.prepareFlyBack(it) }?.also { it.manual = true }
        try {
            progress.collect {
                edge = it.swipeEdge
                state.seekTo(it.progress, below)
                fb?.t = it.progress
            }
            // Сначала доезжаем, потом выкидываем страницу: иначе уходящая мигнёт пустой.
            kotlinx.coroutines.coroutineScope {
                if (fb != null) launch { androidx.compose.animation.core.animate(fb.t, 1f, animationSpec = tween(300, easing = androidx.compose.animation.core.FastOutSlowInEasing)) { v, _ -> fb.t = v } }
                state.animateTo(below)
            }
            nav.back()
            nav.flyBack = null
        } catch (e: kotlin.coroutines.cancellation.CancellationException) {
            kotlinx.coroutines.coroutineScope {
                if (fb != null) launch { androidx.compose.animation.core.animate(fb.t, 0f, animationSpec = tween(220)) { v, _ -> fb.t = v } }
                state.animateTo(state.currentState)
            }
            if (nav.flyBack === fb) nav.flyBack = null
            throw e
        }
    }
    pageTransition.AnimatedContent(
        transitionSpec = {
            val fwd = (targetState?.seq ?: 0) > (initialState?.seq ?: 0)
            if (fwd) {
                (slideInHorizontally { it / 3 } + fadeIn()) togetherWith (slideOutHorizontally { -it / 3 } + fadeOut())
            } else {
                val dir = if (edge == androidx.activity.BackEventCompat.EDGE_RIGHT) -1 else 1
                (slideInHorizontally { -dir * it / 4 } + fadeIn()) togetherWith
                    (slideOutHorizontally { dir * it / 4 } + scaleOut(targetScale = 0.88f) + fadeOut(tween(easing = { f -> (f * 1.6f - 0.6f).coerceIn(0f, 1f) })))
            }.using(SizeTransform(clip = false))
        },
        contentKey = { it?.seq },
    ) { route ->
        if (route != null) {
            // Уходящая назад страница скругляется, пока едет.
            val corner by this.transition.animateDp(label = "corner") {
                if (it == androidx.compose.animation.EnterExitState.PostExit && !fwdOf(pageTransition)) 36.dp else 0.dp
            }
            Box(
                Modifier.fillMaxSize()
                    .clip(RoundedCornerShape(corner))
                    .background(MaterialTheme.colorScheme.background),
            ) {
                PageView(app, nav, route)
            }
        } else {
            Box(Modifier.fillMaxSize())
        }
    }
    FlyOverlay(app, nav)
    FlyBackOverlay(app, nav)
}

/** Обложка нажатой карточки перелетает на своё место на открывшейся странице. */
@Composable
private fun FlyOverlay(app: YaApp, nav: Nav) {
    val fly = nav.fly ?: return
    val to = nav.flyTarget
    val t = remember(fly) { Animatable(0f) }
    // Страница так и не отдала место (ошибка, нет обложки) — не держим.
    LaunchedEffect(fly) {
        kotlinx.coroutines.delay(5000)
        if (nav.fly === fly) nav.fly = null
    }
    LaunchedEffect(fly, to != null) {
        if (to == null) return@LaunchedEffect
        // Картинка страницы к концу полёта уже в кеше — без подмены на лету.
        nav.flyUrl?.let { app.cover(it) }
        t.animateTo(1f, tween(520, easing = androidx.compose.animation.core.FastOutSlowInEasing))
        if (nav.fly === fly) nav.fly = null
    }
    var origin by remember { mutableStateOf(androidx.compose.ui.geometry.Offset.Zero) }
    Box(Modifier.fillMaxSize().onGloballyPositioned { origin = it.positionInRoot() }) {
        if (to != null) {
            val f = fly.tap.from
            val k = t.value
            val d = androidx.compose.ui.platform.LocalDensity.current
            Art(
                app, fly.tap.url,
                Modifier
                    .offset {
                        androidx.compose.ui.unit.IntOffset(
                            (androidx.compose.ui.util.lerp(f.left, to.left, k) - origin.x).roundToInt(),
                            (androidx.compose.ui.util.lerp(f.top, to.top, k) - origin.y).roundToInt(),
                        )
                    }
                    .size(
                        with(d) { androidx.compose.ui.util.lerp(f.width, to.width, k).toDp() },
                        with(d) { androidx.compose.ui.util.lerp(f.height, to.height, k).toDp() },
                    ),
                corner = androidx.compose.ui.unit.lerp(fly.tap.corner, 36.dp, k),
            )
        }
    }
}

/** Обложка возвращается с шапки закрывающейся страницы на свою карточку. */
@Composable
private fun FlyBackOverlay(app: YaApp, nav: Nav) {
    val fb = nav.flyBack ?: return
    LaunchedEffect(fb) {
        if (fb.manual) return@LaunchedEffect
        androidx.compose.animation.core.animate(0f, 1f, animationSpec = tween(420, easing = androidx.compose.animation.core.FastOutSlowInEasing)) { v, _ -> fb.t = v }
        if (nav.flyBack === fb) nav.flyBack = null
    }
    var origin by remember { mutableStateOf(androidx.compose.ui.geometry.Offset.Zero) }
    Box(Modifier.fillMaxSize().onGloballyPositioned { origin = it.positionInRoot() }) {
        val f = fb.from
        val to = fb.tap.from
        val k = fb.t
        val d = androidx.compose.ui.platform.LocalDensity.current
        Art(
            app, fb.tap.url,
            Modifier
                .offset {
                    androidx.compose.ui.unit.IntOffset(
                        (androidx.compose.ui.util.lerp(f.left, to.left, k) - origin.x).roundToInt(),
                        (androidx.compose.ui.util.lerp(f.top, to.top, k) - origin.y).roundToInt(),
                    )
                }
                .size(
                    with(d) { androidx.compose.ui.util.lerp(f.width, to.width, k).toDp() },
                    with(d) { androidx.compose.ui.util.lerp(f.height, to.height, k).toDp() },
                ),
            corner = androidx.compose.ui.unit.lerp(36.dp, fb.tap.corner, k),
        )
    }
}

/** Переход вглубь стека (а не назад). */
private fun fwdOf(t: androidx.compose.animation.core.Transition<Route?>) =
    (t.targetState?.seq ?: 0) > (t.currentState?.seq ?: 0)

@Composable
private fun PageView(app: YaApp, nav: Nav, route: Route) {
    val pages by app.pages.collectAsStateWithLifecycle()
    val src = Source.Page(route.id)
    // Закрытая страница ещё уезжает — держим последний ответ, а не спиннер.
    var last by remember(route.seq) { mutableStateOf<Page?>(null) }
    val fresh = pages[route.id]
    if (fresh != null && fresh !== last) last = fresh
    val shown = fresh ?: last
    // Новое проявляется поверх старого, а старое не гаснет: иначе две одинаковые обложки
    // на середине перехода просвечивают друг сквозь друга и шапка мигает.
    AnimatedContent(
        shown,
        transitionSpec = { fadeIn(tween(280)) togetherWith androidx.compose.animation.ExitTransition.None },
        label = "pageLoad",
    ) { p ->
    when (p) {
        null -> {
            PendingHero(app, nav, route, nav.fly?.takeIf { it.routeId == route.id }?.tap)
        }
        is Page.Failed -> Column(Modifier.fillMaxSize().statusBarsPadding()) {
            TopBar(route.title, nav)
            Empty(p.error)
        }
        is Page.Tracks -> TracksPage(app, nav, p, src, route.share)
        is Page.Artist -> if (route.tab >= 0) ArtistTracks(app, nav, p, src, route) else ArtistPage(app, nav, p, src, route.share)
        is Page.Albums -> Column(Modifier.fillMaxSize().statusBarsPadding()) {
            TopBar(p.title, nav)
            LazyVerticalGrid(GridCells.Adaptive(156.dp), contentPadding = gridPadding()) {
                albumGrid(app, nav, p.albums)
            }
        }
        is Page.Search -> Column(Modifier.fillMaxSize().statusBarsPadding()) {
            TopBar(p.query, nav)
            SearchResults(app, nav, p, src)
        }
    }
    }
}

/** Мягко пульсирующая заготовка на месте ещё не пришедшего содержимого. */
fun Modifier.skeleton(corner: Dp = 12.dp): Modifier = composed {
    val a by androidx.compose.animation.core.rememberInfiniteTransition(label = "skeleton").animateFloat(
        0.45f, 1f,
        androidx.compose.animation.core.infiniteRepeatable(tween(900), androidx.compose.animation.core.RepeatMode.Reverse),
        label = "skeletonAlpha",
    )
    val color = MaterialTheme.colorScheme.surfaceContainerHighest
    drawBehind { drawRoundRect(color, cornerRadius = androidx.compose.ui.geometry.CornerRadius(corner.toPx()), alpha = a) }
}

@Composable
private fun SkeletonRow(i: Int) {
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Box(Modifier.size(52.dp).skeleton(12.dp))
        Spacer(Modifier.width(14.dp))
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Box(Modifier.fillMaxWidth(if (i % 3 == 0) 0.6f else if (i % 3 == 1) 0.45f else 0.72f).height(14.dp).skeleton(7.dp))
            Box(Modifier.fillMaxWidth(if (i % 2 == 0) 0.32f else 0.4f).height(11.dp).skeleton(6.dp))
        }
    }
}

/**
 * Пока список треков грузится, шапка уже стоит на своём месте: обложка известна заранее,
 * так что она летит сразу, а не после ответа сети.
 */
@Composable
private fun PendingHero(app: YaApp, nav: Nav, route: Route, tap: Tap?) {
    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize().statusBarsPadding()) {
            Spacer(Modifier.height(64.dp))
            Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                if (tap == null) Box(Modifier.size(232.dp).skeleton(36.dp)) else Art(
                    app, tap.url,
                    Modifier.size(232.dp)
                        .onGloballyPositioned { c ->
                            val r = c.boundsInRoot()
                            nav.heroRects[route.id] = r
                            if (nav.fly?.routeId == route.id) {
                                if (nav.flyTarget != r) nav.flyTarget = r
                                nav.flyUrl = tap.url
                            }
                        }
                        .graphicsLayer { alpha = if (nav.fly?.routeId == route.id || nav.flyBack?.routeId == route.id) 0f else 1f },
                    corner = 36.dp,
                )
                Spacer(Modifier.height(20.dp))
                Text(
                    route.title,
                    style = MaterialTheme.typography.headlineMedium,
                    fontWeight = FontWeight.Black,
                    textAlign = TextAlign.Center,
                    maxLines = 3,
                    overflow = TextOverflow.Ellipsis,
                )
                Spacer(Modifier.height(24.dp))
            }
            // Список ещё грузится: на месте строк — заготовки, которые потом перетекут в треки.
            Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
                repeat(7) { i -> SkeletonRow(i) }
            }
        }
        ShapeIconButton(
            Ic.Back, "Назад", { nav.back() },
            Modifier.statusBarsPadding().padding(horizontal = 12.dp, vertical = 8.dp),
            size = 48.dp,
            container = MaterialTheme.colorScheme.surfaceContainerHighest.copy(alpha = 0.9f),
        )
    }
}

@Composable
private fun TopBar(title: String, nav: Nav) = TopBar(title) { nav.back() }

@Composable
private fun TopBar(title: String, back: () -> Unit) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        ShapeIconButton(Ic.Back, "Назад", back, size = 48.dp)
        Spacer(Modifier.width(12.dp))
        Text(title, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
fun Empty(text: String, action: (@Composable () -> Unit)? = null) {
    Column(
        Modifier.fillMaxSize().padding(32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(text, color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center)
        action?.let {
            Spacer(Modifier.height(12.dp))
            it()
        }
    }
}

/** Ряд в сетке с отступами выходит за них до края экрана — карточки не обрезаются посреди отступа. */
fun Modifier.bleed(by: Dp) = layout { m, c ->
    val extra = (by * 2).roundToPx()
    val w = c.maxWidth + extra
    val p = m.measure(c.copy(minWidth = w, maxWidth = w))
    layout(c.maxWidth, p.height) { p.place(-extra / 2, 0) }
}

private fun gridPadding() = PaddingValues(start = 16.dp, end = 16.dp, top = 8.dp, bottom = 120.dp)

// ── страница треков: плейлист, альбом, похожие, чарт ────────────────────────

@Composable
private fun TracksPage(app: YaApp, nav: Nav, p: Page.Tracks, src: Source, share: String?) {
    val now = LocalNow.current
    val list = androidx.compose.foundation.lazy.rememberLazyListState()
    val nowAt = remember(p.tracks, now.id) { p.tracks.indexOfFirst { it.id == now.id }.takeIf { it >= 0 } }
    Box(Modifier.fillMaxSize()) {
    LazyColumn(Modifier.fillMaxSize().statusBarsPadding(), state = list, contentPadding = PaddingValues(bottom = 120.dp)) {
        // Место под «Назад»: сама кнопка висит поверх списка и не уезжает с ним.
        item(contentType = "bar") { Spacer(Modifier.height(64.dp)) }
        item(contentType = "hero") {
            Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                if (p.cover != null) {
                    val pid = (src as? Source.Page)?.id
                    Art(
                        app, p.cover,
                        Modifier.size(232.dp)
                            .onGloballyPositioned { c ->
                                val r = c.boundsInRoot()
                                if (pid != null) nav.heroRects[pid] = r
                                // Сюда прилетает обложка нажатой карточки.
                                if (nav.fly?.routeId == pid) {
                                    if (nav.flyTarget != r) nav.flyTarget = r
                                    nav.flyUrl = p.cover
                                }
                            }
                            .graphicsLayer { alpha = if (nav.fly?.routeId == pid || nav.flyBack?.routeId == pid) 0f else 1f },
                        corner = 36.dp,
                    )
                    Spacer(Modifier.height(20.dp))
                }
                Text(
                    p.title,
                    style = MaterialTheme.typography.headlineMedium,
                    fontWeight = FontWeight.Black,
                    textAlign = TextAlign.Center,
                    maxLines = 3,
                    overflow = TextOverflow.Ellipsis,
                )
                val a = p.album
                if (a != null) {
                    p.tracks.firstOrNull()?.artistList?.takeIf { it.isNotEmpty() }?.let {
                        AvatarStack(app, it, 40.dp, MaterialTheme.colorScheme.background)
                        Spacer(Modifier.height(6.dp))
                    }
                    Text(
                        a.artists,
                        style = MaterialTheme.typography.titleMedium,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.clip(RoundedCornerShape(12.dp)).clickable {
                            p.tracks.firstOrNull()?.let { t -> nav.artistOf(t) }
                        }.padding(horizontal = 8.dp, vertical = 2.dp),
                    )
                    Spacer(Modifier.height(8.dp))
                    Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        Chip(a.kind)
                        val full = app.settings.collectAsStateWithLifecycle().value?.fullReleaseDate == true
                        (a.released?.takeIf { full } ?: a.year?.toString())?.let { Chip(it) }
                        if (p.tracks.isNotEmpty()) Chip(tracksWord(p.tracks.size))
                    }
                } else if (p.subtitle.isNotEmpty()) {
                    Text(p.subtitle, color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center)
                }
                val have = p.tracks.count { app.isDownloaded(it.id) }
                if (p.playlist && have > 0) {
                    Spacer(Modifier.height(14.dp))
                    DownloadProgress(have, p.tracks.size)
                }
                Spacer(Modifier.height(16.dp))
                Spacer(Modifier.height(12.dp))
            }
        }
        if (p.tracks.isEmpty()) item { Empty("В этом списке нет треков") }
        itemsIndexed(p.tracks, contentType = { _, _ -> "track" }) { i, t ->
            TrackRow(
                app, t, current = now.id == t.id, playing = now.playing,
                onLongClick = { nav.menu = MenuTarget(t, src, i) },
            ) { app.core.playFrom(src, i.toUInt()) }
        }
    }
    ShapeIconButton(
        Ic.Back, "Назад", { nav.back() },
        Modifier.statusBarsPadding().padding(horizontal = 12.dp, vertical = 8.dp),
        size = 48.dp,
        container = MaterialTheme.colorScheme.surfaceContainerHighest.copy(alpha = 0.9f),
    )
    if (share != null) {
        val ctx = androidx.compose.ui.platform.LocalContext.current
        ShapeIconButton(
            Ic.Share, "Поделиться", { shareText(ctx, share) },
            Modifier.align(Alignment.TopEnd).statusBarsPadding().padding(horizontal = 12.dp, vertical = 8.dp),
            size = 48.dp,
            container = MaterialTheme.colorScheme.surfaceContainerHighest.copy(alpha = 0.9f),
        )
    }
    if (p.tracks.isNotEmpty()) FloatingPlay { app.core.playFrom(src, 0u) }
    ListJumps(app, list, p.tracks.size, 2, nowAt, nowAt?.let { p.tracks[it].thumb })
    }
}

/** Кнопка «играть» внизу справа, над мини-плеером: под большим пальцем, тянуться не надо. */
@Composable
fun BoxScope.FloatingPlay(onClick: () -> Unit) {
    val lift = if (LocalNow.current.id != null) 100.dp else 20.dp
    val pop = remember { Animatable(0f) }
    LaunchedEffect(Unit) { pop.animateTo(1f, spring(dampingRatio = 0.5f, stiffness = 300f)) }
    val liftPx by animateDpAsState(lift, spring(dampingRatio = 0.7f, stiffness = 300f), label = "lift")
    PlayButton(false, 64.dp, onClick = onClick, modifier = Modifier.align(Alignment.BottomEnd).padding(end = 20.dp, bottom = liftPx)
        .graphicsLayer { scaleX = pop.value; scaleY = pop.value })
}

/**
 * Прыжки по длинному списку — столбиком над кнопкой «играть», пока список листают (и пару секунд после):
 * в начало, к играющему треку (его обложка), в конец. `first` — индекс первого трека среди элементов списка.
 */
@Composable
fun BoxScope.ListJumps(app: YaApp, state: androidx.compose.foundation.lazy.LazyListState, count: Int, first: Int, now: Int?, nowThumb: String?) {
    if (count < 20) return
    val scope = rememberCoroutineScope()
    var awake by remember { mutableStateOf(false) }
    LaunchedEffect(state.isScrollInProgress) {
        if (state.isScrollInProgress) awake = true
        else {
            kotlinx.coroutines.delay(2500)
            awake = false
        }
    }
    val visible = state.layoutInfo.visibleItemsInfo
    val nowShown = now != null && visible.any { it.index == first + now }
    val lift = if (LocalNow.current.id != null) 100.dp else 20.dp
    androidx.compose.animation.AnimatedVisibility(
        awake,
        Modifier.align(Alignment.BottomEnd).padding(end = 24.dp, bottom = lift + 64.dp + 12.dp),
        enter = fadeIn() + androidx.compose.animation.scaleIn(initialScale = 0.8f),
        exit = fadeOut() + scaleOut(targetScale = 0.8f),
    ) {
        val bg = MaterialTheme.colorScheme.surfaceContainerHighest
        Column(
            Modifier.shadow(8.dp, RoundedCornerShape(28.dp)).clip(RoundedCornerShape(28.dp)).background(bg).padding(4.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            fun jump(i: Int) = scope.launch { state.scrollToItem(i) }
            ShapeIconButton(Ic.Down, "В начало", { jump(0) }, Modifier.graphicsLayer { rotationZ = 180f }, size = 48.dp, container = Color.Transparent)
            if (now != null && !nowShown) {
                // Концентрично: 28 − 4 = 24.
                Box(
                    Modifier.size(48.dp).clip(RoundedCornerShape(24.dp)).clickable { jump((first + now - 3).coerceAtLeast(0)) },
                    contentAlignment = Alignment.Center,
                ) {
                    Art(app, nowThumb, Modifier.size(40.dp), corner = 20.dp)
                    EqBars(LocalNow.current.playing, Color.White)
                }
            }
            ShapeIconButton(Ic.Down, "В конец", { jump(first + count - 1) }, size = 48.dp, container = Color.Transparent)
        }
    }
}

@Composable
private fun Chip(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.onSecondaryContainer,
        modifier = Modifier.appear().clip(RoundedCornerShape(12.dp)).background(MaterialTheme.colorScheme.secondaryContainer)
            .padding(horizontal = 10.dp, vertical = 4.dp),
    )
}

fun tracksWord(n: Int): String {
    val w = when {
        n % 10 == 1 && n % 100 != 11 -> "трек"
        n % 10 in 2..4 && n % 100 !in 12..14 -> "трека"
        else -> "треков"
    }
    return "$n $w"
}

// ── исполнитель ─────────────────────────────────────────────────────────────

/**
 * Исполнитель: его фото во всю ширину до самого верха экрана, имя крупно поверх низа фото;
 * под ним — волна по нему, популярное с местами, дискография, похожие, об исполнителе.
 */
@Composable
private fun ArtistPage(app: YaApp, nav: Nav, p: Page.Artist, src: Source, share: String?) {
    val now = LocalNow.current
    var tab by rememberSaveable { mutableIntStateOf(0) }
    val bg = MaterialTheme.colorScheme.background
    val listState = androidx.compose.foundation.lazy.rememberLazyListState()
    Box(Modifier.fillMaxSize()) {
    LazyColumn(Modifier.fillMaxSize(), state = listState, contentPadding = PaddingValues(bottom = 120.dp)) {
        item(contentType = "hero") {
            BoxWithConstraints(Modifier.fillMaxWidth()) {
                val h = minOf(maxWidth, 460.dp)
                Box(Modifier.fillMaxWidth().height(h).clip(androidx.compose.ui.graphics.RectangleShape)) {
                    // Параллакс: фото уезжает вдвое медленнее списка и чуть темнеет.
                    Art(app, p.artist.photo, Modifier.fillMaxSize().graphicsLayer {
                        val off = if (listState.firstVisibleItemIndex == 0) listState.firstVisibleItemScrollOffset.toFloat() else size.height
                        translationY = off * 0.5f
                        val k = 1f + (off / size.height) * 0.12f
                        scaleX = k; scaleY = k
                        alpha = 1f - (off / size.height) * 0.6f
                    }, corner = 0.dp)
                    // Фото растворяется в фон, имя читается поверх.
                    Box(
                        Modifier.fillMaxSize().background(
                            androidx.compose.ui.graphics.Brush.verticalGradient(
                                0f to Color.Black.copy(alpha = 0.35f),
                                0.22f to Color.Transparent,
                                0.5f to Color.Transparent,
                                1f to bg,
                            ),
                        ),
                    )
                    Text(
                        p.artist.name,
                        style = MaterialTheme.typography.displayLarge.copy(
                            shadow = androidx.compose.ui.graphics.Shadow(Color.Black.copy(alpha = 0.5f), blurRadius = 24f),
                        ),
                        fontWeight = FontWeight.Black,
                        maxLines = 3,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.align(Alignment.BottomStart).padding(horizontal = 20.dp).appear(),
                    )
                }
            }
        }
        item(contentType = "meta") {
            Column(Modifier.fillMaxWidth().padding(start = 20.dp, end = 20.dp, top = 6.dp)) {
                p.listeners?.let {
                    Text("${grouped(it.toLong())} слушателей за месяц", style = MaterialTheme.typography.titleMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                if (p.genres.isNotEmpty()) {
                    Text(
                        p.genres.take(3).joinToString(", ").replaceFirstChar { it.uppercase() },
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                if (p.artist.id.isNotEmpty()) {
                    Spacer(Modifier.height(16.dp))
                    Row(
                        Modifier.height(52.dp).clip(RoundedCornerShape(26.dp)).background(MaterialTheme.colorScheme.primaryContainer)
                            .clickable { app.core.startWaveWith(listOf("artist:${p.artist.id}"), "Волна по «${p.artist.name}»") }
                            .padding(start = 18.dp, end = 22.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Icon(Ic.Wave, null, tint = MaterialTheme.colorScheme.onPrimaryContainer, modifier = Modifier.size(22.dp))
                        Spacer(Modifier.width(10.dp))
                        Text("Волна по исполнителю", style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.Bold,
                            color = MaterialTheme.colorScheme.onPrimaryContainer)
                    }
                }
            }
        }
        // Популярное, своё и услышанное в волне — отдельными вкладками, каждая со всем списком.
        val tabs = artistTabs(p)
        if (tabs.isNotEmpty()) {
            val cur = tab.coerceIn(0, tabs.size - 1)
            val (_, shown, base) = tabs[cur]
            if (tabs.size > 1) {
                item(contentType = "tabs") {
                    Segments(tabs.map { it.first }, cur, { tab = it }, Modifier.fillMaxWidth().padding(start = 20.dp, end = 20.dp, top = 24.dp, bottom = 8.dp))
                }
            } else {
                item { Section(tabs[0].first, 20.dp) }
            }
            itemsIndexed(shown.take(ARTIST_PREVIEW), key = { i, t -> "$cur:$i:${t.id}" }, contentType = { _, _ -> "track" }) { i, t ->
                val at = base + i
                TrackRow(
                    app, t, current = now.id == t.id, playing = now.playing, rank = if (cur == 0 && tabs[0].first == "Популярное") i + 1 else null,
                    onLongClick = { nav.menu = MenuTarget(t, src, at) },
                ) { app.core.playFrom(src, at.toUInt()) }
            }
            if (shown.size > ARTIST_PREVIEW) {
                item(contentType = "more") {
                    val pid = (src as? Source.Page)?.id
                    Row(
                        Modifier.padding(start = 20.dp, top = 8.dp).height(44.dp).clip(RoundedCornerShape(22.dp))
                            .background(MaterialTheme.colorScheme.surfaceContainerHighest)
                            .clickable(enabled = pid != null) { nav.artistTracks(pid!!, cur, p.artist.name) }
                            .padding(horizontal = 20.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text("Показать все · ${shown.size}", style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.Bold)
                    }
                }
            }
        }
        albumRow(app, nav, "Дискография", p.albums, 20.dp)
        albumRow(app, nav, "С участием", p.also, 20.dp)
        if (p.similar.isNotEmpty()) {
            item { Section("Похожие исполнители", 20.dp) }
            item {
                LazyRow(contentPadding = PaddingValues(horizontal = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    items(p.similar, key = { it.id }) { a -> ArtistCircle(app, a) { nav.artist(a.id, a.name) } }
                }
            }
        }
        p.about?.let { about ->
            item { Section("Об исполнителе", 20.dp) }
            item {
                var open by rememberSaveable { mutableStateOf(false) }
                Text(
                    about,
                    maxLines = if (open) Int.MAX_VALUE else 5,
                    overflow = TextOverflow.Ellipsis,
                    style = MaterialTheme.typography.bodyLarge.copy(lineHeight = 24.sp),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 12.dp).clip(RoundedCornerShape(16.dp))
                        .clickable { open = !open }.padding(8.dp),
                )
            }
        }
    }
    // «Назад» поверх фото: кнопка в подложке, чтобы читалась на любом снимке.
    ShapeIconButton(
        Ic.Back, "Назад", { nav.back() },
        Modifier.statusBarsPadding().padding(12.dp),
        size = 48.dp,
        container = Color.Black.copy(alpha = 0.35f),
        tint = Color.White,
    )
    if (share != null) {
        val ctx = androidx.compose.ui.platform.LocalContext.current
        ShapeIconButton(
            Ic.Share, "Поделиться", { shareText(ctx, share) },
            Modifier.align(Alignment.TopEnd).statusBarsPadding().padding(12.dp),
            size = 48.dp,
            container = Color.Black.copy(alpha = 0.35f),
            tint = Color.White,
        )
    }
    if (p.tracks.isNotEmpty() || p.collection.isNotEmpty() || p.wave.isNotEmpty()) FloatingPlay { app.core.playFrom(src, 0u) }
    }
}

private const val ARTIST_PREVIEW = 5

/** Вкладки исполнителя: название, треки, смещение в общем списке источника. */
private fun artistTabs(p: Page.Artist) = listOfNotNull(
    p.tracks.takeIf { it.isNotEmpty() }?.let { Triple("Популярное", it, 0) },
    p.collection.takeIf { it.isNotEmpty() }?.let { Triple("В коллекции", it, p.tracks.size) },
    p.wave.takeIf { it.isNotEmpty() }?.let { Triple("В волне", it, p.tracks.size + p.collection.size) },
)

/** Весь список одной вкладки исполнителя. */
@Composable
private fun ArtistTracks(app: YaApp, nav: Nav, p: Page.Artist, src: Source, route: Route) {
    val now = LocalNow.current
    val (name, shown, base) = artistTabs(p).getOrNull(route.tab) ?: return Empty("Пусто")
    Column(Modifier.fillMaxSize().statusBarsPadding()) {
        TopBar("$name · ${route.title}", nav)
        Box(Modifier.fillMaxSize()) {
            LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 120.dp)) {
                itemsIndexed(shown, key = { i, t -> "$i:${t.id}" }, contentType = { _, _ -> "track" }) { i, t ->
                    val at = base + i
                    TrackRow(
                        app, t, current = now.id == t.id, playing = now.playing, rank = if (name == "Популярное") i + 1 else null,
                        onLongClick = { nav.menu = MenuTarget(t, src, at) },
                    ) { app.core.playFrom(src, at.toUInt()) }
                }
            }
            FloatingPlay { app.core.playFrom(src, base.toUInt()) }
        }
    }
}

private fun grouped(n: Long): String = "%,d".format(n).replace(',', ' ')

@Composable
fun Section(title: String, inset: Dp = 24.dp) {
    Text(
        title,
        style = MaterialTheme.typography.titleLarge,
        fontWeight = FontWeight.Bold,
        modifier = Modifier.padding(start = inset, end = inset, top = 24.dp, bottom = 8.dp),
    )
}

private fun LazyListScope.albumRow(app: YaApp, nav: Nav, title: String, albums: List<AlbumItem>, inset: Dp = 24.dp) {
    if (albums.isEmpty()) return
    item { Section(title, inset) }
    item {
        LazyRow(contentPadding = PaddingValues(horizontal = inset - 8.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            items(albums, key = { it.id }) { a -> AlbumCard(app, a, 148.dp) { nav.album(a.id, a.title) } }
        }
    }
}

fun LazyGridScope.albumGrid(app: YaApp, nav: Nav, albums: List<AlbumItem>) {
    items(albums, key = { it.id }) { a -> AlbumCard(app, a, null) { nav.album(a.id, a.title) } }
}

/** Обложка альбома и подписи; `width == null` — по ширине ячейки сетки. */
@Composable
fun AlbumCard(app: YaApp, a: AlbumItem, width: Dp?, onClick: () -> Unit) {
    val (cover, tap) = rememberCoverTap(a.cover)
    // Концентрично: обложка 20 + отступ 6 = 26.
    Column(
        (if (width != null) Modifier.width(width + 12.dp) else Modifier.fillMaxWidth())
            .bounceClick(RoundedCornerShape(26.dp), onClick = { tap(); onClick() }).padding(6.dp),
    ) {
        Art(app, a.cover, (if (width != null) Modifier.size(width) else Modifier.fillMaxWidth().aspectRatio(1f)).then(cover), corner = 20.dp)
        Spacer(Modifier.height(8.dp))
        Text(a.title, style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.Bold, maxLines = 2, overflow = TextOverflow.Ellipsis)
        Text(a.artists, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(a.kind, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.primary)
            a.year?.let { Text(it.toString(), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
        }
    }
}

@Composable
fun PlaylistCard(app: YaApp, p: PlaylistItem, width: Dp?, onClick: () -> Unit) {
    val (cover, tap) = rememberCoverTap(p.cover)
    Column(
        (if (width != null) Modifier.width(width + 12.dp) else Modifier.fillMaxWidth())
            .bounceClick(RoundedCornerShape(26.dp), onClick = { tap(); onClick() }).padding(6.dp),
    ) {
        Box(
            if (width != null) Modifier.size(width) else Modifier.fillMaxWidth().aspectRatio(1f),
            contentAlignment = Alignment.Center,
        ) {
            Art(app, p.cover, Modifier.fillMaxSize().then(cover), corner = 20.dp)
            if (p.cover == null) Icon(Ic.Queue, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(40.dp))
        }
        Spacer(Modifier.height(8.dp))
        Text(p.title, style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.Bold, maxLines = 2, overflow = TextOverflow.Ellipsis)
        Text(
            p.description ?: p.owner,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        if (p.trackCount > 0u) {
            Text(tracksWord(p.trackCount.toInt()), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.primary)
        }
    }
}

@Composable
fun ArtistCircle(app: YaApp, a: ArtistItem, onClick: () -> Unit) {
    Column(
        Modifier.width(100.dp).bounceClick(RoundedCornerShape(24.dp), onClick = onClick).padding(vertical = 6.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Art(app, a.photo, Modifier.size(84.dp), corner = 42.dp)
        Spacer(Modifier.height(6.dp))
        Text(a.name, style = MaterialTheme.typography.labelLarge, maxLines = 2, overflow = TextOverflow.Ellipsis, textAlign = TextAlign.Center)
    }
}

// ── поиск ───────────────────────────────────────────────────────────────────

@Composable
fun SearchTab(app: YaApp, nav: Nav) {
    var query by rememberSaveable { mutableStateOf("") }
    var pageId by rememberSaveable { mutableStateOf<Long?>(null) }
    val pages by app.pages.collectAsStateWithLifecycle()
    val settings by app.settings.collectAsStateWithLifecycle()
    fun run(q: String) {
        val text = q.trim()
        if (text.isEmpty()) return
        query = text
        pageId?.let { app.closePage(it.toULong()) }
        pageId = app.core.search(text).toLong()
    }
    // Строка поиска — внизу, под большим пальцем: над мини-плеером, а с клавиатурой — прямо над ней.
    val density = LocalDensity.current
    val ime = WindowInsets.ime.getBottom(density)
    var gap by remember { mutableIntStateOf(0) }
    val above = if (LocalNow.current.id != null) 92.dp else 12.dp
    val lift = with(density) { maxOf(above.roundToPx(), ime - gap + 8.dp.roundToPx()).toDp() }
    Box(
        Modifier.fillMaxSize().onGloballyPositioned { c ->
            // Сколько от низа вкладки до низа экрана (навигация и системная полоса) — клавиатура считается от низа экрана.
            val root = c.findRootCoordinates().size.height
            gap = root - c.boundsInRoot().bottom.toInt()
        },
    ) {
        Column(Modifier.fillMaxSize().statusBarsPadding()) {
            Header("Поиск", "") {}
            val id = pageId?.toULong()
            when (val p = id?.let { pages[it] }) {
                null -> if (id != null) {
                    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { LoadingIndicator(Modifier.size(72.dp)) }
                } else {
                    SearchHistory(app, settings?.searchHistory.orEmpty(), ::run)
                }
                is Page.Search -> SearchResults(app, nav, p, Source.Page(id))
                is Page.Failed -> Empty(p.error) { TextButton(onClick = { run(query) }) { Text("Повторить") } }
                else -> {}
            }
        }
        Box(Modifier.align(Alignment.BottomCenter).padding(bottom = lift)) {
            SearchPill(query, {
                query = it
                if (it.isEmpty()) pageId?.let { id -> app.closePage(id.toULong()); pageId = null }
            }, "Треки, альбомы, исполнители", onSubmit = { run(query) }, floating = true)
        }
    }
}

@Composable
private fun SearchHistory(app: YaApp, history: List<String>, run: (String) -> Unit) {
    if (history.isEmpty()) {
        Empty("Найдите трек, альбом или исполнителя")
        return
    }
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 190.dp)) {
        item { Section("Недавние запросы") }
        items(history, key = { it }) { q ->
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 8.dp).clip(RoundedCornerShape(20.dp)).clickable { run(q) }
                    .padding(start = 16.dp, end = 4.dp, top = 4.dp, bottom = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(Ic.History, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(20.dp))
                Spacer(Modifier.width(14.dp))
                Text(q, Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                ShapeIconButton(Ic.Close, "Забыть", { app.core.forgetSearch(q) }, size = 48.dp, container = Color.Transparent,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

@Composable
private fun SearchResults(app: YaApp, nav: Nav, p: Page.Search, src: Source) {
    val now = LocalNow.current
    if (p.tracks.isEmpty() && p.albums.isEmpty() && p.artists.isEmpty() && p.playlists.isEmpty()) {
        Empty("Ничего не нашлось по запросу «${p.query}»")
        return
    }
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 190.dp)) {
        if (p.artists.isNotEmpty()) {
            item { Section("Исполнители") }
            item {
                LazyRow(contentPadding = PaddingValues(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    items(p.artists, key = { it.id }) { a -> ArtistCircle(app, a) { nav.artist(a.id, a.name) } }
                }
            }
        }
        if (p.tracks.isNotEmpty()) {
            item { Section("Треки") }
            itemsIndexed(p.tracks, contentType = { _, _ -> "track" }) { i, t ->
                TrackRow(
                    app, t, current = now.id == t.id, playing = now.playing,
                    onLongClick = { nav.menu = MenuTarget(t, src, i) },
                ) { app.core.playFrom(src, i.toUInt()) }
            }
        }
        albumRow(app, nav, "Альбомы", p.albums)
        if (p.playlists.isNotEmpty()) {
            item { Section("Плейлисты") }
            item {
                LazyRow(contentPadding = PaddingValues(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    items(p.playlists, key = { it.uid + ":" + it.kind }) { pl -> PlaylistCard(app, pl, 148.dp) { nav.playlist(pl) } }
                }
            }
        }
    }
}

// ── новинки ─────────────────────────────────────────────────────────────────

@Composable
fun ReleasesTab(app: YaApp, nav: Nav) {
    val pages by app.pages.collectAsStateWithLifecycle()
    val (_, personal) = app.playlists.collectAsStateWithLifecycle().value
    LaunchedEffect(Unit) { if (app.releasesId == null) app.releasesId = app.core.releases() }
    val page = app.releasesId?.let { pages[it] }
    LazyVerticalGrid(GridCells.Adaptive(156.dp), Modifier.fillMaxSize().statusBarsPadding(), contentPadding = gridPadding()) {
        fun full(block: @Composable () -> Unit) = item(span = { GridItemSpan(maxLineSpan) }) { block() }
        full { Header("Новинки", "") {} }
        full {
            // Карточка чарта: акцентная плашка, концентричная внутренней иконке (28 = 16 + 12).
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 6.dp, vertical = 4.dp).clip(RoundedCornerShape(28.dp))
                    .background(MaterialTheme.colorScheme.primaryContainer).clickable { nav.chart() }.padding(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Box(
                    Modifier.size(52.dp).clip(RoundedCornerShape(16.dp)).background(MaterialTheme.colorScheme.primary),
                    contentAlignment = Alignment.Center,
                ) { Icon(Ic.Chart, null, tint = MaterialTheme.colorScheme.onPrimary) }
                Spacer(Modifier.width(14.dp))
                Column {
                    Text("Чарт", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
                    Text("Популярное прямо сейчас", color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        }
        if (personal.isNotEmpty()) {
            full { Section("Настроили для вас", 8.dp) }
            full {
                LazyRow(Modifier.bleed(16.dp), contentPadding = PaddingValues(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    items(personal, key = { it.uid + ":" + it.kind }) { pl -> PlaylistCard(app, pl, 148.dp) { nav.playlist(pl) } }
                }
            }
        }
        full { Section("Новые релизы", 8.dp) }
        when (page) {
            null -> full { Box(Modifier.fillMaxWidth().height(200.dp), contentAlignment = Alignment.Center) { LoadingIndicator() } }
            is Page.Albums -> albumGrid(app, nav, page.albums)
            is Page.Failed -> full {
                Box(Modifier.height(200.dp)) {
                    Empty(page.error) {
                        TextButton(onClick = {
                            app.releasesId?.let(app::closePage)
                            app.releasesId = app.core.releases()
                        }) { Text("Повторить") }
                    }
                }
            }
            else -> {}
        }
    }
}

// ── коллекция: плейлисты ────────────────────────────────────────────────────

@Composable
fun PlaylistsGrid(app: YaApp, nav: Nav) {
    val (own, personal) = app.playlists.collectAsStateWithLifecycle().value
    if (own.isEmpty() && personal.isEmpty()) {
        Empty("Плейлисты, которые вы создадите или сохраните в Яндекс Музыке, появятся здесь")
        return
    }
    LazyVerticalGrid(GridCells.Adaptive(156.dp), Modifier.fillMaxSize(), contentPadding = gridPadding()) {
        fun full(block: @Composable () -> Unit) = item(span = { GridItemSpan(maxLineSpan) }) { block() }
        if (own.isNotEmpty()) {
            full { Section("Мои плейлисты", 8.dp) }
            items(own, key = { "o" + it.uid + ":" + it.kind }) { pl -> PlaylistCard(app, pl, null) { nav.playlist(pl) } }
        }
        if (personal.isNotEmpty()) {
            full { Section("Настроили для вас", 8.dp) }
            items(personal, key = { "p" + it.uid + ":" + it.kind }) { pl -> PlaylistCard(app, pl, null) { nav.playlist(pl) } }
        }
    }
}

/**
 * Скачанное: сверху — сколько места занято (своё и подобранное для офлайн-волны — двумя цветами
 * одной полосы, против лимита докачки), ниже — треки, недавно игранные первыми. Работает без сети.
 */
@Composable
fun Downloaded(app: YaApp, nav: Nav) {
    // Снимок библиотеки берётся при каждом заходе на сегмент; ушли — страница в ядре закрывается.
    val id = remember { app.core.downloaded() }
    DisposableEffect(id) { onDispose { app.closePage(id) } }
    val pages by app.pages.collectAsStateWithLifecycle()
    val st by app.settings.collectAsStateWithLifecycle()
    val now = LocalNow.current
    val p = pages[id] as? Page.Tracks
    val tracks = p?.tracks.orEmpty()
    val src = Source.Page(id)
    val list = androidx.compose.foundation.lazy.rememberLazyListState()
    val nowAt = remember(tracks, now.id) { tracks.indexOfFirst { it.id == now.id }.takeIf { it >= 0 } }
    Box(Modifier.fillMaxSize()) {
        LazyColumn(Modifier.fillMaxSize(), state = list, contentPadding = PaddingValues(bottom = 120.dp)) {
            item(contentType = "storage") { st?.let { StorageBar(it) } }
            when {
                p == null -> item { Box(Modifier.fillMaxWidth().padding(48.dp), contentAlignment = Alignment.Center) { LoadingIndicator() } }
                tracks.isEmpty() -> item {
                    Empty("Здесь появится то, что вы слушали: каждый сыгранный трек сохраняется и потом играет без сети")
                }
                else -> itemsIndexed(tracks, contentType = { _, _ -> "track" }) { i, t ->
                    TrackRow(
                        app, t, current = now.id == t.id, playing = now.playing,
                        onLongClick = { nav.menu = MenuTarget(t, src, i) },
                    ) { app.core.playFrom(src, i.toUInt()) }
                }
            }
        }
        if (tracks.isNotEmpty()) FloatingPlay { app.core.playFrom(src, 0u) }
        ListJumps(app, list, tracks.size, 1, nowAt, nowAt?.let { tracks[it].thumb })
    }
}

/** Полоса места: всё скачанное, из него подобранное для офлайн-волны — другим цветом. */
@Composable
private fun StorageBar(s: SettingsData) {
    val cs = MaterialTheme.colorScheme
    fun gb(v: Float) = "%.1f".format(v)
    Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 12.dp)) {
        Row(verticalAlignment = Alignment.Bottom) {
            Icon(Ic.Download, null, Modifier.size(28.dp), tint = cs.primary)
            Spacer(Modifier.width(10.dp))
            Text("${gb(s.libraryGb)} ГБ", style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.Black)
            Spacer(Modifier.width(8.dp))
            Text(tracksWord(s.libraryTracks.toInt()), style = MaterialTheme.typography.bodyMedium, color = cs.onSurfaceVariant,
                modifier = Modifier.padding(bottom = 3.dp))
        }
        Spacer(Modifier.height(10.dp))
        // Шкала — до большего из «всё скачанное» и «всё скачанное + свободный лимит докачки».
        val own = (s.libraryGb - s.smartUsedGb).coerceAtLeast(0f)
        val scale = maxOf(s.libraryGb, own + if (s.smartDownload) s.smartLimitGb else 0f, 0.1f)
        val ownF by animateFloatAsState(own / scale, label = "own")
        val smartF by animateFloatAsState(s.smartUsedGb / scale, label = "smart")
        val ownColor = cs.primary
        val smartColor = cs.tertiary
        val track = cs.surfaceContainerHighest
        Canvas(Modifier.fillMaxWidth().height(12.dp).clip(RoundedCornerShape(6.dp))) {
            drawRect(track)
            drawRect(ownColor, size = androidx.compose.ui.geometry.Size(size.width * ownF, size.height))
            drawRect(
                smartColor,
                topLeft = androidx.compose.ui.geometry.Offset(size.width * ownF, 0f),
                size = androidx.compose.ui.geometry.Size(size.width * smartF, size.height),
            )
        }
        Spacer(Modifier.height(8.dp))
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Legend(ownColor, "Сыгранное и своё, ${gb(own)} ГБ")
            if (s.smartDownload || s.smartUsedGb > 0f) Legend(smartColor, "Для офлайн-волны, ${gb(s.smartUsedGb)} из ${s.smartLimitGb.toInt()} ГБ")
        }
    }
}

@Composable
private fun Legend(color: Color, text: String) {
    // Точка — у первой строки, даже если подпись переносится.
    Row(verticalAlignment = Alignment.Top) {
        Box(Modifier.padding(top = 5.dp).size(8.dp).clip(RoundedCornerShape(4.dp)).background(color))
        Spacer(Modifier.width(8.dp))
        Text(text, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** Выбор одного из вариантов: связанная группа кнопок MD3E (выбранная — полностью круглая). */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun Segments(options: List<String>, selected: Int, onSelect: (Int) -> Unit, modifier: Modifier = Modifier) {
    Row(modifier, horizontalArrangement = Arrangement.spacedBy(ButtonGroupDefaults.ConnectedSpaceBetween)) {
        options.forEachIndexed { i, o ->
            ToggleButton(
                checked = i == selected,
                onCheckedChange = { onSelect(i) },
                modifier = Modifier.weight(1f),
                shapes = when (i) {
                    0 -> ButtonGroupDefaults.connectedLeadingButtonShapes()
                    options.lastIndex -> ButtonGroupDefaults.connectedTrailingButtonShapes()
                    else -> ButtonGroupDefaults.connectedMiddleButtonShapes()
                },
                // Невыбранные кнопки — на ступень выше карточки, иначе сливаются с ней.
                colors = ToggleButtonDefaults.colors(
                    containerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                    contentColor = MaterialTheme.colorScheme.onSurface,
                    checkedContainerColor = MaterialTheme.colorScheme.primary,
                    checkedContentColor = MaterialTheme.colorScheme.onPrimary,
                ),
                contentPadding = PaddingValues(horizontal = 4.dp),
            ) {
                Text(o, style = MaterialTheme.typography.labelLarge, maxLines = 1, overflow = TextOverflow.Clip)
            }
        }
    }
}

// ── настройки ───────────────────────────────────────────────────────────────

private val QUALITIES = listOf("lossless" to "Lossless", "auto" to "Авто", "hq" to "Высокое", "nq" to "Экономия")
private val CROSSFADES = listOf(0f, 3f, 6f, 10f)
private val SMART_LIMITS = listOf(1f, 2f, 5f, 10f, 20f)

@Composable
fun SettingsTab(app: YaApp, sub: Boolean, setSub: (Boolean) -> Unit, pull: () -> Float) {
    val st by app.settings.collectAsStateWithLifecycle()
    val s: SettingsData = st ?: return
    AnimatedContent(
        sub,
        // Жест «назад» из подменю чуть сдвигает его за пальцем.
        modifier = Modifier.graphicsLayer { translationX = pull() * size.width * 0.12f },
        transitionSpec = {
            val dir = if (targetState) 1 else -1
            (slideInHorizontally { it * dir / 4 } + fadeIn()) togetherWith (slideOutHorizontally { -it * dir / 4 } + fadeOut())
        },
        label = "settingsSub",
    ) { inSub ->
        if (inSub) OfflineSettings(app, s) { setSub(false) } else MainSettings(app, s) { setSub(true) }
    }
}

@Composable
private fun MainSettings(app: YaApp, s: SettingsData, openSub: () -> Unit) {
    Column(Modifier.fillMaxSize().statusBarsPadding().verticalScroll(rememberScrollState()).padding(bottom = 120.dp)) {
        Header("Настройки", "") {}
        Section("Звук") {
            Group("Качество", "Lossless — FLAC без потерь, Авто — выбирает Яндекс по скорости сети, Высокое — AAC 256, Экономия — для мобильного интернета") {
                Segments(QUALITIES.map { it.second }, QUALITIES.indexOfFirst { it.first == s.quality }.coerceAtLeast(0), {
                    app.core.setQuality(QUALITIES[it].first)
                }, Modifier.fillMaxWidth())
            }
            Group("Плавный переход", "Наплыв из одного трека в другой; «Выкл» — встык") {
                Segments(
                    CROSSFADES.map { if (it == 0f) "Выкл" else "${it.toInt()} с" },
                    CROSSFADES.indexOfFirst { kotlin.math.abs(it - s.crossfadeSecs) < 0.5f }.coerceAtLeast(0),
                    { app.core.setCrossfade(CROSSFADES[it]) },
                    Modifier.fillMaxWidth(),
                )
            }
            Group(null, null) {
                Toggle("Выравнивание громкости", "Выключено — звук bit-perfect, как в исходнике", s.normalize) { app.core.setNormalize(it) }
                if (s.normalize) {
                    var lufs by remember(s.targetLufs) { mutableFloatStateOf(s.targetLufs) }
                    Spacer(Modifier.height(12.dp))
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text("Целевая громкость", Modifier.weight(1f))
                        Text("${lufs.roundToInt()} LUFS", color = MaterialTheme.colorScheme.primary, fontWeight = FontWeight.Bold)
                    }
                    Slider(
                        lufs,
                        { lufs = it.roundToInt().toFloat() },
                        valueRange = -23f..-8f,
                        steps = 14,
                        onValueChangeFinished = { app.core.setTargetLufs(lufs) },
                    )
                }
            }
            Group(null, null) {
                Toggle("Эквалайзер", "Выключен — звук идёт без обработки (bit-perfect)", s.eqEnabled) { app.core.setEqEnabled(it) }
                androidx.compose.animation.AnimatedVisibility(s.eqEnabled) {
                    Column {
                        Spacer(Modifier.height(12.dp))
                        Segments(EQ_BANDS.map { "$it" }, EQ_BANDS.indexOf(s.eqGains.size).coerceAtLeast(0), {
                            app.core.setEqBands(EQ_BANDS[it].toUInt())
                        }, Modifier.fillMaxWidth())
                        Spacer(Modifier.height(8.dp))
                        EqBands(app.core, s.eqGains, s.eqEnabled)
                        Spacer(Modifier.height(4.dp))
                        EqReset(app.core, s.eqGains)
                    }
                }
            }
        }
        Section("Воспроизведение") {
            Group(null, null) {
                Toggle("Быстрый старт", "Из сети трек начинает играть в AAC, lossless подменяет его без паузы; следующие треки прогреваются заранее", s.fastStart) {
                    app.core.setFastStart(it)
                }
            }
            Group(null, null) {
                Toggle("Волна после списка", "Плейлист, альбом или «Мне нравится» доиграли — дальше волна по ним, а не тишина", s.waveAfterList) {
                    app.core.setWaveAfterList(it)
                }
            }
            Group(null, null) {
                Toggle("История прослушиваний", "Отправлять в Яндекс: так точнее рекомендации волны", s.reportPlays) { app.core.setReportPlays(it) }
            }
        }
        Section("Устройства и сеть") {
            Group(null, null) {
                SubmenuRow(
                    "Офлайн и докачка",
                    when {
                        s.offlineMode -> "Сейчас офлайн: играет только скачанное"
                        s.smartDownload -> "Треки для офлайн-волны докачиваются: ${"%.1f".format(s.smartUsedGb)} из ${s.smartLimitGb.toInt()} ГБ"
                        else -> "Офлайн-режим, докачка треков для офлайн-волны"
                    },
                ) { openSub() }
            }
            Group(null, null) {
                Toggle("Слушать на других устройствах", s.ynisonStatus.replaceFirstChar { it.uppercase() }.let {
                    if (s.ynison) "Ynison: $it" else "Телефон или компьютер с Яндекс Музыкой включает музыку здесь, а отсюда — там"
                }, s.ynison) { app.core.setYnison(it) }
            }
            Group(null, null) {
                val ctx = androidx.compose.ui.platform.LocalContext.current
                // Проверяем заново при каждом возвращении на экран (решение принимается в системном окне).
                var free by remember { mutableStateOf(unrestricted(ctx)) }
                androidx.lifecycle.compose.LifecycleResumeEffect(Unit) {
                    free = unrestricted(ctx)
                    onPauseOrDispose {}
                }
                Toggle(
                    "Работа в фоне",
                    if (free) "Система не ограничивает приложение — музыку в фоне не прервёт"
                    else "Система может остановить музыку в фоне ради батареи. Включите, чтобы этого не было",
                    free,
                ) { if (!free) askBackground(ctx, onlyOnce = false) }
            }
        }
        Section("Вид") {
            Group("Фон плеера", "Блоб — мягкие пятна цвета обложки, Спектр — столбики по звучанию, Оба — столбики поверх блоба") {
                val kinds = listOf("blob", "spectrum", "both")
                Segments(listOf("Блоб", "Спектр", "Оба"), kinds.indexOf(s.visualizer).coerceAtLeast(0), {
                    app.core.setVisualizer(kinds[it])
                }, Modifier.fillMaxWidth())
            }
            Group(null, null) {
                Toggle("Полная дата выхода", "В шапке альбома — день, месяц и год, а не только год", s.fullReleaseDate) {
                    app.core.setFullReleaseDate(it)
                }
            }
        }
        val ctx = androidx.compose.ui.platform.LocalContext.current
        val version = remember { runCatching { ctx.packageManager.getPackageInfo(ctx.packageName, 0).versionName }.getOrNull() }
        Text(
            "yamusicrs ${version.orEmpty()}",
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.fillMaxWidth().padding(24.dp),
            textAlign = TextAlign.Center,
        )
    }
}

private val SMART_WINDOWS = listOf("Всегда", "Ночью", "Свои часы")

/** Офлайн-режим и докачка для офлайн-волны: подменю настроек. «Назад» — сверху и жестом. */
@Composable
private fun OfflineSettings(app: YaApp, s: SettingsData, back: () -> Unit) {
    Column(Modifier.fillMaxSize().statusBarsPadding()) {
        TopBar("Офлайн и докачка", back)
        var confirm by remember { mutableStateOf<Boolean?>(null) }
        confirm?.let { smartOnly ->
            AlertDialog(
                onDismissRequest = { confirm = null },
                title = { Text(if (smartOnly) "Удалить подобранное?" else "Удалить всё скачанное?") },
                text = {
                    Text(
                        if (smartOnly) "Исчезнут треки, которые Яндекс подобрал для офлайн-волны. Сыгранное и своё останется."
                        else "Все скачанные треки пропадут с телефона, без сети играть будет нечего. Они скачаются снова, когда вы их включите.",
                    )
                },
                confirmButton = { TextButton(onClick = { app.core.clearDownloads(smartOnly); confirm = null }) { Text("Удалить") } },
                dismissButton = { TextButton(onClick = { confirm = null }) { Text("Отмена") } },
            )
        }
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = 120.dp)) {
            Section("Скачанное") { Group(
                null,
                "${tracksWord(s.libraryTracks.toInt())}, ${"%.1f".format(s.libraryGb)} ГБ; из них подобрано для офлайн-волны ${"%.1f".format(s.smartUsedGb)} ГБ",
            ) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedButton(onClick = { confirm = true }, enabled = s.smartUsedGb > 0f) { Text("Удалить подобранное") }
                    OutlinedButton(onClick = { confirm = false }, enabled = s.libraryTracks > 0u) { Text("Удалить всё") }
                }
            } }
            Section("Режим") { Group(null, null) {
                Toggle(
                    "Офлайн-режим",
                    "Без сети по своей воле: Коллекция, плейлисты и офлайн-волна из скачанного",
                    s.offlineMode,
                ) { app.core.setOfflineMode(it) }
            } }
            Section("Докачка для офлайн-волны") { Group(null, null) {
                Toggle(
                    "Докачивать для офлайн-волны",
                    "Яндекс подбирает треки, которые может выбрать офлайн-волна, и они тихо скачиваются. " +
                        "Подобранное, но не сыгранное, он может потом попросить удалить; своё не удаляется",
                    s.smartDownload,
                ) { app.core.setSmartDownload(it) }
            }
            androidx.compose.animation.AnimatedVisibility(s.smartDownload) {
                Column(verticalArrangement = Arrangement.spacedBy(22.dp)) {
                    Group("Качество докачки", "Отдельно от качества стрима. «Авто» — как решит Яндекс по скорости сети") {
                        Segments(QUALITIES.map { it.second }, QUALITIES.indexOfFirst { it.first == s.smartQuality }.coerceAtLeast(0), {
                            app.core.setSmartQuality(QUALITIES[it].first)
                        }, Modifier.fillMaxWidth())
                    }
                    Group("Место", "Сколько отдать подобранным трекам; занято ${"%.1f".format(s.smartUsedGb)} ГБ") {
                        Segments(
                            SMART_LIMITS.map { "${it.toInt()} ГБ" },
                            SMART_LIMITS.indexOfFirst { it >= s.smartLimitGb - 0.01f }.let { if (it < 0) SMART_LIMITS.size - 1 else it },
                            { app.core.setSmartLimit(SMART_LIMITS[it]) },
                            Modifier.fillMaxWidth(),
                        )
                    }
                    val from = s.smartFromHour.toInt()
                    val to = s.smartToHour.toInt()
                    val mode = when {
                        from == to -> 0
                        from == 1 && to == 7 -> 1
                        else -> 2
                    }
                    Group(
                        "Когда качать",
                        when (mode) {
                            0 -> "В любое время, как только Яндекс подберёт треки"
                            1 -> "С 1 до 7 ночи — пока телефон заряжается и не нужен"
                            else -> "С $from:00 до $to:00 по местному времени"
                        },
                    ) {
                        Segments(SMART_WINDOWS, mode, {
                            when (it) {
                                0 -> app.core.setSmartWindow(0u, 0u)
                                1 -> app.core.setSmartWindow(1u, 7u)
                                else -> if (mode != 2) app.core.setSmartWindow(22u, 8u)
                            }
                        }, Modifier.fillMaxWidth())
                        if (mode == 2) {
                            Spacer(Modifier.height(12.dp))
                            HourSlider("С", from) { app.core.setSmartWindow(it.toUByte(), to.toUByte()) }
                            HourSlider("До", to) { app.core.setSmartWindow(from.toUByte(), it.toUByte()) }
                        }
                    }
                    Group(null, null) {
                        Toggle("Только по Wi-Fi", "По мобильной сети не качать — как в официальном приложении", s.smartWifiOnly) {
                            app.core.setSmartWifiOnly(it)
                        }
                    }
                }
            } }
        }
    }
}

/** Час суток ползунком; отпустили — значение уходит в ядро. */
@Composable
private fun HourSlider(label: String, hour: Int, set: (Int) -> Unit) {
    var h by remember(hour) { mutableFloatStateOf(hour.toFloat()) }
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text(label, Modifier.width(32.dp), style = MaterialTheme.typography.labelLarge)
        Slider(h, { h = it.roundToInt().toFloat() }, Modifier.weight(1f), valueRange = 0f..23f, steps = 22,
            onValueChangeFinished = { set(h.toInt()) })
        Text("${h.toInt()}:00", Modifier.width(52.dp), textAlign = TextAlign.End, color = MaterialTheme.colorScheme.primary,
            fontWeight = FontWeight.Bold)
    }
}

/** Пункт, открывающий подменю: заголовок, что там сейчас, стрелка. */
@Composable
private fun SubmenuRow(title: String, hint: String, open: () -> Unit) {
    Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).clickable(onClick = open), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
            Text(hint, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Spacer(Modifier.width(12.dp))
        Icon(Ic.Back, null, Modifier.size(24.dp).graphicsLayer { rotationZ = 180f }, tint = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** Раздел настроек: подпись и один скруглённый контейнер, внутри пункты через равные промежутки. */
@Composable
private fun Section(title: String, content: @Composable ColumnScope.() -> Unit) {
    Text(
        title,
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.padding(start = 32.dp, end = 24.dp, top = 20.dp, bottom = 8.dp),
    )
    Surface(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp),
        shape = MaterialTheme.shapes.extraLarge,
        color = MaterialTheme.colorScheme.surfaceContainer,
    ) {
        Column(Modifier.fillMaxWidth().padding(20.dp), verticalArrangement = Arrangement.spacedBy(22.dp), content = content)
    }
}

/** Пункт внутри раздела: название, пояснение, затем сам элемент управления. */
@Composable
private fun Group(title: String?, hint: String?, content: @Composable ColumnScope.() -> Unit) {
    Column(Modifier.fillMaxWidth()) {
        title?.let { Text(it, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold) }
        hint?.let {
            Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Spacer(Modifier.height(12.dp))
        }
        content()
    }
}

@Composable
private fun Toggle(title: String, hint: String, on: Boolean, set: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth().clickable { set(!on) }, verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
            Text(hint, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Spacer(Modifier.width(12.dp))
        Switch(
            on, set,
            thumbContent = if (on) ({ Icon(Ic.Check, null, Modifier.size(SwitchDefaults.IconSize)) }) else null,
        )
    }
}

// ── меню трека ──────────────────────────────────────────────────────────────

/** Долгое нажатие на трек: в очередь, лайк, переходы к исполнителю, альбому, похожим. */
@Composable
fun TrackMenu(app: YaApp, nav: Nav) {
    val m = nav.menu ?: return
    val likes by app.likes.collectAsStateWithLifecycle()
    val lyr by app.lyrics.collectAsStateWithLifecycle()
    val hasLyrics = lyr?.let { it.trackId == m.track.id && it.lyrics != null } == true
    val liked = remember(likes, m.track.id) { likes.any { it.id == m.track.id } }
    val t = m.track
    ModalBottomSheet(
        onDismissRequest = { nav.menu = null },
        containerColor = MaterialTheme.colorScheme.surfaceContainer,
        shape = RoundedCornerShape(topStart = 32.dp, topEnd = 32.dp),
    ) {
        Column(Modifier.padding(bottom = 24.dp)) {
            Row(Modifier.padding(horizontal = 24.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Art(app, t.thumb, Modifier.size(56.dp), corner = 16.dp)
                Spacer(Modifier.width(14.dp))
                Column(Modifier.weight(1f)) {
                    Text(t.title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(t.artists, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                if (t.artistList.isNotEmpty()) {
                    Spacer(Modifier.width(12.dp))
                    AvatarStack(app, t.artistList, 36.dp, MaterialTheme.colorScheme.surfaceContainer, max = 3)
                }
            }
            Spacer(Modifier.height(8.dp))
            fun close() { nav.menu = null }
            if (m.source !is Source.Queue) {
                MenuItem(Ic.PlayNext, "Играть следующим") { app.core.enqueueFrom(m.source, m.index.toUInt(), true); close() }
                MenuItem(Ic.AddQueue, "Добавить в очередь") { app.core.enqueueFrom(m.source, m.index.toUInt(), false); close() }
            }
            MenuItem(if (liked) Ic.Heart else Ic.HeartOutline, if (liked) "Убрать из «Мне нравится»" else "Мне нравится") {
                app.core.likeFrom(m.source, m.index.toUInt()); close()
            }
            // Один пункт на всех: у нескольких исполнителей он открывает их список (подменю).
            when {
                t.artistList.size > 1 -> MenuItem(Ic.Person, "Исполнители") { nav.artistOf(t) }
                t.artistList.size == 1 -> t.artistList[0].let { a -> MenuItem(Ic.Person, a.name, lead = { Avatar(app, a, 36.dp) }) { nav.artist(a.id, a.name) } }
                t.artistId != null -> MenuItem(Ic.Person, t.artists) { nav.artist(t.artistId, t.artists) }
            }
            if (t.albumId != null) MenuItem(Ic.Album, t.album.ifEmpty { "Альбом" }) { nav.album(t.albumId, t.album) }
            MenuItem(Ic.Similar, "Похожие треки") { nav.similar(t) }
            val ctx = androidx.compose.ui.platform.LocalContext.current
            MenuItem(Ic.Share, "Поделиться") {
                shareText(ctx, if (t.albumId != null) "/album/${t.albumId}/track/${t.id}" else "/track/${t.id}"); close()
            }
            if (hasLyrics) MenuItem(Ic.Lyrics, "Текст песни целиком") { close(); nav.lyrics = true }
        }
    }
}

/** У трека несколько исполнителей — к какому перейти. */
@Composable
fun ArtistPicker(app: YaApp, nav: Nav) {
    val list = nav.artists ?: return
    ModalBottomSheet(
        onDismissRequest = { nav.artists = null },
        containerColor = MaterialTheme.colorScheme.surfaceContainer,
        shape = RoundedCornerShape(topStart = 32.dp, topEnd = 32.dp),
    ) {
        Column(Modifier.padding(bottom = 24.dp)) {
            Row(Modifier.padding(horizontal = 24.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Text("Исполнители", Modifier.weight(1f), style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.Black)
                AvatarStack(app, list, 40.dp, MaterialTheme.colorScheme.surfaceContainer, max = 4)
            }
            list.forEach { a -> MenuItem(Ic.Person, a.name, lead = { Avatar(app, a, 40.dp) }) { nav.artist(a.id, a.name) } }
        }
    }
}

/** Текст играющего трека целиком: выделяется пальцем, копируется кнопкой. */
@Composable
fun LyricsSheet(app: YaApp, nav: Nav) {
    if (!nav.lyrics) return
    val lyr by app.lyrics.collectAsStateWithLifecycle()
    val l = lyr?.lyrics
    if (l == null) {
        nav.lyrics = false
        return
    }
    val text = remember(l) { l.lines.joinToString("\n") { it.text } }
    val clip = androidx.compose.ui.platform.LocalClipboardManager.current
    var copied by remember(l) { mutableStateOf(false) }
    ModalBottomSheet(
        onDismissRequest = { nav.lyrics = false },
        containerColor = MaterialTheme.colorScheme.surfaceContainer,
        shape = RoundedCornerShape(topStart = 32.dp, topEnd = 32.dp),
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    ) {
        Column(Modifier.fillMaxWidth()) {
            Row(Modifier.fillMaxWidth().padding(start = 24.dp, end = 16.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Text("Текст песни", Modifier.weight(1f), style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.Black)
                FilledTonalButton(onClick = {
                    clip.setText(androidx.compose.ui.text.AnnotatedString(text))
                    copied = true
                }) { Text(if (copied) "Скопировано" else "Скопировать") }
            }
            androidx.compose.foundation.text.selection.SelectionContainer {
                Text(
                    text,
                    style = MaterialTheme.typography.bodyLarge.copy(lineHeight = 26.sp),
                    modifier = Modifier.fillMaxWidth().verticalScroll(rememberScrollState())
                        .padding(start = 24.dp, end = 24.dp, top = 8.dp, bottom = 32.dp),
                )
            }
        }
    }
}

@Composable
private fun MenuItem(icon: ImageVector, text: String, lead: (@Composable () -> Unit)? = null, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 8.dp).clip(RoundedCornerShape(20.dp)).clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (lead != null) {
            Box(Modifier.size(24.dp).wrapContentSize(unbounded = true)) { lead() }
        } else {
            Icon(icon, null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(24.dp))
        }
        Spacer(Modifier.width(18.dp))
        Text(text, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

// ── очередь (лист со сцены) ─────────────────────────────────────────────────

@Composable
fun QueueSheet(app: YaApp, nav: Nav, queuePos: Int?) {
    if (!nav.queue) return
    val q by app.queue.collectAsStateWithLifecycle()
    val now = LocalNow.current
    val (title, items) = q
    val list = androidx.compose.foundation.lazy.rememberLazyListState(((queuePos ?: 0) - 2).coerceAtLeast(0))
    ModalBottomSheet(
        onDismissRequest = { nav.queue = false },
        containerColor = MaterialTheme.colorScheme.surfaceContainer,
        shape = RoundedCornerShape(topStart = 32.dp, topEnd = 32.dp),
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    ) {
        Header("Очередь", title) {}
        if (items.isEmpty()) {
            Box(Modifier.fillMaxWidth().height(200.dp)) { Empty("Включите волну, альбом или плейлист — треки встанут сюда") }
            return@ModalBottomSheet
        }
        LazyColumn(Modifier.fillMaxWidth(), state = list, contentPadding = PaddingValues(bottom = 24.dp)) {
            itemsIndexed(items, contentType = { _, _ -> "track" }) { i, t ->
                TrackRow(
                    app, t, current = queuePos == i && now.id == t.id, playing = now.playing,
                    dim = queuePos?.let { i < it } == true,
                    onLongClick = { nav.menu = MenuTarget(t, Source.Queue, i) },
                ) { app.core.playFrom(Source.Queue, i.toUInt()) }
            }
        }
    }
}

// ── Ynison: устройства и пульт ──────────────────────────────────────────────

private fun deviceIcon(kind: String) = when (kind) {
    "ANDROID", "IOS" -> Ic.Phone
    "WEB", "WEB_DESKTOP" -> Ic.Computer
    "SMART_SPEAKER" -> Ic.Speaker
    else -> Ic.Cast
}

/** «Слушать на…»: где играет музыка, тап — перевести туда. */
@Composable
fun DevicesSheet(app: YaApp, nav: Nav, s: uniffi.yamusic_android.PlayerState) {
    if (!nav.devices) return
    val st by app.settings.collectAsStateWithLifecycle()
    val here = s.remote == null && s.devices.none { it.active }
    ModalBottomSheet(
        onDismissRequest = { nav.devices = false },
        containerColor = MaterialTheme.colorScheme.surfaceContainer,
        shape = RoundedCornerShape(topStart = 32.dp, topEnd = 32.dp),
    ) {
        Column(Modifier.padding(bottom = 24.dp)) {
            Text(
                "Слушать на",
                style = MaterialTheme.typography.headlineSmall,
                fontWeight = FontWeight.Black,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp),
            )
            DeviceRow(Ic.Phone, "Этот телефон", if (here) "Сейчас здесь" else null, here, here && !s.paused) {
                app.core.transfer("")
                nav.devices = false
            }
            s.devices.filter { it.canPlay }.forEach { d ->
                DeviceRow(deviceIcon(d.kind), d.title.ifBlank { "Устройство без имени" }, if (d.active) "Сейчас здесь" else null, d.active,
                    d.active && s.remote?.paused == false) {
                    app.core.transfer(d.id)
                    nav.devices = false
                }
            }
            val settings = st
            if (settings != null && (!settings.ynison || s.devices.isEmpty())) {
                Text(
                    if (!settings.ynison) "Включите «Слушать на других устройствах» в настройках"
                    else settings.ynisonStatus.replaceFirstChar { it.uppercase() },
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 24.dp, vertical = 12.dp),
                )
            }
        }
    }
}

@Composable
private fun DeviceRow(icon: ImageVector, title: String, sub: String?, active: Boolean, playing: Boolean, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 8.dp).clip(RoundedCornerShape(20.dp))
            .background(if (active) MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.6f) else Color.Transparent)
            .clickable(onClick = onClick).padding(horizontal = 16.dp, vertical = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, null, tint = if (active) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(26.dp))
        Spacer(Modifier.width(18.dp))
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyLarge, fontWeight = if (active) FontWeight.Bold else FontWeight.Medium)
            sub?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.primary) }
        }
        if (active) EqBars(playing, MaterialTheme.colorScheme.primary)
    }
}

/**
 * Пульт: музыка играет на другом устройстве — пилюля вместо мини-плеера. Обложка и название
 * оттуда, кнопки управляют им; тап — выбрать, где слушать.
 */
@Composable
fun RemotePill(app: YaApp, nav: Nav, r: uniffi.yamusic_android.RemoteNow) {
    // Позиция тикает от последнего ответа хаба.
    val clock = remember(r.positionMs, r.paused) { System.currentTimeMillis() }
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(r.paused) {
        while (!r.paused) {
            kotlinx.coroutines.delay(500)
            now = System.currentTimeMillis()
        }
    }
    val pos = if (r.paused) r.positionMs.toLong() else r.positionMs.toLong() + (now - clock).coerceAtLeast(0)
    val frac = if (r.durationMs > 0u) (pos.toFloat() / r.durationMs.toFloat()).coerceIn(0f, 1f) else 0f
    val fill = MaterialTheme.colorScheme.primary.copy(alpha = 0.14f)
    Surface(
        shape = RoundedCornerShape(32.dp),
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        shadowElevation = 12.dp,
        modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 10.dp).clickable { nav.devices = true },
    ) {
        Box {
            Box(Modifier.matchParentSize().drawBehindFill(frac, fill))
            Row(Modifier.padding(8.dp), verticalAlignment = Alignment.CenterVertically) {
                Box(contentAlignment = Alignment.BottomEnd) {
                    Art(app, r.cover, Modifier.size(52.dp), corner = 24.dp)
                    Icon(Ic.Cast, null, tint = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.size(20.dp).clip(RoundedCornerShape(6.dp)).background(MaterialTheme.colorScheme.surfaceContainerHigh).padding(2.dp))
                }
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Text(r.title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text("Играет на «${r.device}»", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.primary,
                        maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                if (r.hasPrev) ShapeIconButton(Ic.Prev, "Назад", { app.core.remotePrev() }, size = 44.dp, container = Color.Transparent)
                PlayButton(!r.paused, 52.dp, onClick = { app.core.remotePlayPause() })
                if (r.hasNext) ShapeIconButton(Ic.Next, "Дальше", { app.core.remoteNext() }, size = 44.dp, container = Color.Transparent)
            }
        }
    }
}

private fun Modifier.drawBehindFill(frac: Float, color: Color) = this.then(
    Modifier.drawBehind { drawRect(color, size = androidx.compose.ui.geometry.Size(size.width * frac, size.height)) },
)
