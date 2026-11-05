package xyz.neodustry.yamusicrs

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.SizeTransform
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.spring
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlin.math.abs
import kotlin.math.sign
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import uniffi.yamusic_android.ToastAction

private data class Shown(val id: Long, val toast: Toast)

/**
 * Один тост за раз: новый сразу сменяет старый (без очереди по 4 с на каждый).
 * Смахивается вбок, закрывается тапом; у тоста с действием — кнопка.
 */
@Composable
fun ToastHost(app: YaApp, modifier: Modifier = Modifier) {
    var shown by remember { mutableStateOf<Shown?>(null) }
    LaunchedEffect(Unit) {
        var n = 0L
        app.toasts.collect { shown = Shown(n++, it) }
    }
    // Таймер перезапускается на каждый новый тост.
    LaunchedEffect(shown?.id) {
        val t = shown?.toast ?: return@LaunchedEffect
        delay(
            when {
                t.action != null -> 6000L
                t.error -> 4500L
                else -> 2500L
            },
        )
        shown = null
    }
    AnimatedContent(
        targetState = shown,
        modifier = modifier.fillMaxWidth().padding(horizontal = 16.dp),
        contentAlignment = Alignment.BottomCenter,
        transitionSpec = {
            (slideInVertically(spring(dampingRatio = 0.75f, stiffness = 500f)) { it / 2 } + fadeIn() + scaleIn(initialScale = 0.92f))
                .togetherWith(fadeOut() + scaleOut(targetScale = 0.92f) + slideOutVertically { it / 3 })
                .using(SizeTransform(clip = false))
        },
        contentKey = { it?.id },
        label = "toast",
    ) { s ->
        if (s == null) {
            Box(Modifier)
        } else {
            ToastCard(
                s.toast,
                onDismiss = { if (shown?.id == s.id) shown = null },
                onAction = { a ->
                    app.core.toastAction(a)
                    if (shown?.id == s.id) shown = null
                },
            )
        }
    }
}

@Composable
private fun ToastCard(t: Toast, onDismiss: () -> Unit, onAction: (ToastAction) -> Unit) {
    val dx = remember { Animatable(0f) }
    val scope = rememberCoroutineScope()
    val cs = MaterialTheme.colorScheme
    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
        Surface(
            shape = RoundedCornerShape(24.dp),
            color = if (t.error) cs.errorContainer else cs.inverseSurface,
            contentColor = if (t.error) cs.onErrorContainer else cs.inverseOnSurface,
            shadowElevation = 6.dp,
            modifier = Modifier
                .widthIn(max = 520.dp)
                .graphicsLayer {
                    translationX = dx.value
                    alpha = (1f - abs(dx.value) / (size.width * 0.8f)).coerceIn(0f, 1f)
                }
                .pointerInput(Unit) {
                    val vt = VelocityTracker()
                    detectHorizontalDragGestures(
                        onDragStart = { vt.resetTracking() },
                        onHorizontalDrag = { ch, d ->
                            vt.addPosition(ch.uptimeMillis, ch.position)
                            scope.launch { dx.snapTo(dx.value + d) }
                        },
                        onDragEnd = {
                            val v = vt.calculateVelocity().x
                            val far = abs(dx.value) > size.width * 0.3f || abs(v) > 1200f
                            scope.launch {
                                if (far) {
                                    val dir = if (abs(v) > 1200f) sign(v) else sign(dx.value)
                                    dx.animateTo(dir * size.width * 1.2f, spring(stiffness = 800f), v)
                                    onDismiss()
                                } else {
                                    dx.animateTo(0f, spring(dampingRatio = 0.7f, stiffness = 600f), v)
                                }
                            }
                        },
                        onDragCancel = { scope.launch { dx.animateTo(0f) } },
                    )
                }
                .clickable(remember { MutableInteractionSource() }, indication = null, onClick = onDismiss),
        ) {
            Row(
                Modifier.padding(start = 20.dp, end = if (t.action != null) 8.dp else 20.dp, top = 8.dp, bottom = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    t.text,
                    style = MaterialTheme.typography.bodyMedium,
                    maxLines = 3,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f, fill = false).padding(vertical = 8.dp),
                )
                t.action?.let { a ->
                    Spacer(Modifier.width(12.dp))
                    FilledTonalButton(
                        onClick = { onAction(a) },
                        colors = ButtonDefaults.filledTonalButtonColors(
                            containerColor = if (t.error) cs.error else cs.inversePrimary,
                            contentColor = if (t.error) cs.onError else cs.onPrimaryContainer,
                        ),
                    ) {
                        Text(
                            when (a) {
                                ToastAction.RESUME -> "Играть"
                                ToastAction.BIT_PERFECT -> "Bit-perfect"
                            },
                        )
                    }
                }
            }
        }
    }
}
