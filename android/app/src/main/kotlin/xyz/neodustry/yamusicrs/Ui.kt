package xyz.neodustry.yamusicrs

import android.graphics.Bitmap
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SecondaryTabRow
import androidx.compose.material3.Slider
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Tab
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import uniffi.yamusic_android.Phase
import uniffi.yamusic_android.PlayerState
import uniffi.yamusic_android.TrackItem

private val Accent = Color(0xFFFFCC00)

@Composable
fun YaTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = darkColorScheme(
            primary = Accent,
            onPrimary = Color.Black,
            background = Color(0xFF0E0E0E),
            surface = Color(0xFF0E0E0E),
            surfaceContainer = Color(0xFF1A1A1A),
        ),
        content = content,
    )
}

@Composable
fun AppScreen(app: YaApp, openUrl: (String) -> Unit) {
    val state by app.state.collectAsStateWithLifecycle()
    val snack = remember { SnackbarHostState() }
    LaunchedEffect(Unit) {
        app.toasts.collect { snack.showSnackbar(it.text) }
    }
    Scaffold(
        snackbarHost = { SnackbarHost(snack) },
        containerColor = MaterialTheme.colorScheme.background,
    ) { pad ->
        Box(Modifier.padding(pad).fillMaxSize()) {
            when (val p = state?.phase) {
                null, is Phase.Connecting -> Connecting((p as? Phase.Connecting)?.error)
                is Phase.Login -> Login(app, p, openUrl)
                is Phase.Main -> Main(app, state!!)
            }
        }
    }
}

@Composable
private fun Connecting(error: String?) {
    Column(
        Modifier.fillMaxSize().padding(32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        CircularProgressIndicator()
        Spacer(Modifier.height(16.dp))
        Text("Подключаюсь…")
        if (error != null) {
            Spacer(Modifier.height(8.dp))
            Text(error, color = MaterialTheme.colorScheme.error, fontSize = 13.sp)
        }
    }
}

@Composable
private fun Login(app: YaApp, p: Phase.Login, openUrl: (String) -> Unit) {
    var token by rememberSaveable { mutableStateOf("") }
    Column(
        Modifier.fillMaxSize().padding(24.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text("Вход в Яндекс Музыку", fontSize = 22.sp, fontWeight = FontWeight.SemiBold)
        Spacer(Modifier.height(24.dp))
        val code = p.code
        val url = p.url
        if (code != null && url != null) {
            Text("Откройте $url и введите код")
            Spacer(Modifier.height(12.dp))
            Text(code, fontSize = 36.sp, fontWeight = FontWeight.Bold, color = Accent, letterSpacing = 4.sp)
            Spacer(Modifier.height(12.dp))
            Button(onClick = { openUrl(url) }) { Text("Открыть страницу входа") }
        } else {
            Text(p.status)
        }
        p.error?.let {
            Spacer(Modifier.height(8.dp))
            Text(it, color = MaterialTheme.colorScheme.error)
        }
        TextButton(onClick = { app.core.newLoginCode() }) { Text("Новый код") }
        Spacer(Modifier.height(24.dp))
        OutlinedTextField(
            value = token,
            onValueChange = { token = it },
            label = { Text("…или вставьте токен") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        Spacer(Modifier.height(8.dp))
        OutlinedButton(onClick = { app.core.submitToken(token.trim()) }, enabled = token.isNotBlank()) {
            Text("Войти по токену")
        }
    }
}

@Composable
private fun Main(app: YaApp, s: PlayerState) {
    var tab by rememberSaveable { mutableIntStateOf(0) }
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(
                if (s.offline) "yamusicrs · офлайн" else "yamusicrs",
                fontSize = 20.sp,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.weight(1f),
            )
            Button(onClick = { app.core.startWave() }) { Text("Моя волна") }
        }
        SecondaryTabRow(selectedTabIndex = tab) {
            Tab(selected = tab == 0, onClick = { tab = 0 }, text = { Text("Мне нравится") })
            Tab(selected = tab == 1, onClick = { tab = 1 }, text = { Text("Очередь") })
        }
        Box(Modifier.weight(1f)) {
            when (tab) {
                0 -> Likes(app, s)
                else -> Queue(app, s)
            }
        }
        if (s.track != null) MiniPlayer(app, s)
    }
}

@Composable
private fun Likes(app: YaApp, s: PlayerState) {
    val likes by app.likes.collectAsStateWithLifecycle()
    var filter by rememberSaveable { mutableStateOf("") }
    if (likes.isEmpty()) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            if (s.likesLoading) CircularProgressIndicator() else Text("Пусто")
        }
        return
    }
    val shown = remember(likes, filter) {
        val q = filter.trim().lowercase()
        likes.withIndex().filter { (_, t) ->
            q.isEmpty() || t.title.lowercase().contains(q) || t.artists.lowercase().contains(q)
        }
    }
    Column {
        OutlinedTextField(
            value = filter,
            onValueChange = { filter = it },
            placeholder = { Text("Поиск по ${likes.size} трекам") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 6.dp),
        )
        LazyColumn(Modifier.fillMaxSize()) {
            itemsIndexed(shown, key = { _, it -> it.index }) { _, (i, t) ->
                TrackRow(t, current = s.track?.id == t.id) { app.core.playLikes(i.toUInt()) }
            }
        }
    }
}

@Composable
private fun Queue(app: YaApp, s: PlayerState) {
    val q by app.queue.collectAsStateWithLifecycle()
    val (title, items) = q
    if (items.isEmpty()) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text("Очередь пуста") }
        return
    }
    val list = rememberLazyListState()
    LaunchedEffect(title, items.size) {
        s.queuePos?.let { list.scrollToItem(it.toInt()) }
    }
    Column {
        Text(title, Modifier.padding(horizontal = 16.dp, vertical = 8.dp), color = MaterialTheme.colorScheme.onSurfaceVariant)
        LazyColumn(Modifier.fillMaxSize(), state = list) {
            itemsIndexed(items) { i, t ->
                TrackRow(t, current = s.queuePos?.toInt() == i) { app.core.playQueue(i.toUInt()) }
            }
        }
    }
}

@Composable
private fun TrackRow(t: TrackItem, current: Boolean, onClick: () -> Unit) {
    val alpha = if (t.available) 1f else 0.4f
    Row(
        Modifier.fillMaxWidth().clickable(enabled = t.available, onClick = onClick).padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(
                t.title,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                color = (if (current) Accent else MaterialTheme.colorScheme.onSurface).copy(alpha = alpha),
                fontWeight = if (current) FontWeight.SemiBold else FontWeight.Normal,
            )
            Text(
                t.artists,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                fontSize = 13.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = alpha),
            )
        }
        Text(time(t.durationMs.toLong()), fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun MiniPlayer(app: YaApp, s: PlayerState) {
    val t = s.track ?: return
    Surface(color = MaterialTheme.colorScheme.surfaceContainer) {
        Column(Modifier.fillMaxWidth().padding(12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Cover(app, t.cover, Modifier.size(56.dp))
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Text(t.title, maxLines = 1, overflow = TextOverflow.Ellipsis, fontWeight = FontWeight.SemiBold)
                    Text(t.artists, maxLines = 1, overflow = TextOverflow.Ellipsis, fontSize = 13.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                    val info = if (s.loading) "загрузка…" else s.format
                    if (info.isNotEmpty()) Text(info, fontSize = 11.sp, color = Accent)
                }
                Glyph(if (s.liked) "♥" else "♡", if (s.liked) Accent else MaterialTheme.colorScheme.onSurface) {
                    app.core.toggleLike()
                }
            }
            Seek(app, s, t.durationMs.toLong())
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceEvenly, verticalAlignment = Alignment.CenterVertically) {
                Glyph("⊘", MaterialTheme.colorScheme.onSurfaceVariant) { app.core.dislike() }
                Glyph("⏮") { app.core.prev() }
                Glyph(if (s.paused) "▶" else "⏸", Accent, big = true) { app.core.playPause() }
                Glyph("⏭") { app.core.next() }
                Spacer(Modifier.size(48.dp))
            }
        }
    }
}

@Composable
private fun Seek(app: YaApp, s: PlayerState, durationMs: Long) {
    var dragging by remember { mutableStateOf(false) }
    var drag by remember { mutableFloatStateOf(0f) }
    val dur = durationMs.coerceAtLeast(1)
    val pos = if (dragging) drag else (s.positionMs.toLong().toFloat() / dur).coerceIn(0f, 1f)
    Column {
        Slider(
            value = pos,
            onValueChange = { dragging = true; drag = it },
            onValueChangeFinished = {
                app.core.seek((drag * dur).toLong().toULong())
                dragging = false
            },
        )
        Row(Modifier.fillMaxWidth()) {
            Text(time((pos * dur).toLong()), fontSize = 11.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Spacer(Modifier.weight(1f))
            Text(time(durationMs), fontSize = 11.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
private fun Glyph(text: String, color: Color = MaterialTheme.colorScheme.onSurface, big: Boolean = false, onClick: () -> Unit) {
    Box(
        Modifier.size(if (big) 64.dp else 48.dp).clip(RoundedCornerShape(50)).clickable(onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Text(text, color = color, fontSize = if (big) 32.sp else 24.sp)
    }
}

@Composable
private fun Cover(app: YaApp, url: String?, modifier: Modifier) {
    val bmp by produceState<Bitmap?>(null, url) { value = url?.let { app.cover(it) } }
    Box(modifier.clip(RoundedCornerShape(6.dp)).background(Color(0xFF2A2A2A))) {
        bmp?.let {
            Image(it.asImageBitmap(), contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
        }
    }
}

private fun time(ms: Long): String {
    val s = ms / 1000
    return "%d:%02d".format(s / 60, s % 60)
}
