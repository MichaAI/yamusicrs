package xyz.neodustry.yamusicrs

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import uniffi.yamusic_android.ArtistItem

/** Фото исполнителя: из самого ответа или доспрошенное у ядра (до прихода — `null`). */
@Composable
fun artistPhoto(app: YaApp, a: ArtistItem): String? {
    LaunchedEffect(a.id) { if (a.photo == null) app.askPhotos(listOf(a.id)) }
    return a.photo ?: app.artistPhotos[a.id]
}

/** Круглая аватарка; пока фото нет — первая буква имени на тонированном круге. */
@Composable
fun Avatar(app: YaApp, a: ArtistItem, size: Dp, modifier: Modifier = Modifier) {
    val url = artistPhoto(app, a)
    Box(
        modifier.size(size).clip(CircleShape).background(MaterialTheme.colorScheme.secondaryContainer),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            a.name.trim().take(1).uppercase(),
            fontWeight = FontWeight.Bold,
            fontSize = (size.value * 0.42f).sp,
            color = MaterialTheme.colorScheme.onSecondaryContainer,
        )
        if (url != null) Art(app, url, Modifier.fillMaxSize(), corner = size / 2)
    }
}

/**
 * Стопка аватарок внахлёст, каждая в кольце цвета фона; больше `max` — кружок «+N».
 * Выпрыгивают по одной, слева направо.
 */
@Composable
fun AvatarStack(app: YaApp, list: List<ArtistItem>, size: Dp, ring: Color, modifier: Modifier = Modifier, max: Int = 4) {
    if (list.isEmpty()) return
    val shown = list.take(max)
    val extra = list.size - shown.size
    Row(modifier, horizontalArrangement = Arrangement.spacedBy(-(size * 0.32f))) {
        shown.forEachIndexed { i, a ->
            Popped(i) { Ringed(size, ring) { Avatar(app, a, size - 4.dp) } }
        }
        if (extra > 0) {
            Popped(shown.size) {
                Ringed(size, ring) {
                    Box(
                        Modifier.size(size - 4.dp).clip(CircleShape).background(MaterialTheme.colorScheme.surfaceContainerHighest),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text("+$extra", fontWeight = FontWeight.Bold, fontSize = (size.value * 0.34f).sp)
                    }
                }
            }
        }
    }
}

@Composable
private fun Ringed(size: Dp, ring: Color, content: @Composable () -> Unit) {
    Box(Modifier.size(size).border(2.dp, ring, CircleShape).padding(2.dp), contentAlignment = Alignment.Center) { content() }
}

@Composable
private fun Popped(index: Int, content: @Composable () -> Unit) {
    val s = remember { Animatable(0f) }
    LaunchedEffect(Unit) {
        delay(index * 70L)
        s.animateTo(1f, spring(dampingRatio = 0.5f, stiffness = 380f))
    }
    Box(Modifier.graphicsLayer { scaleX = s.value; scaleY = s.value }) { content() }
}
