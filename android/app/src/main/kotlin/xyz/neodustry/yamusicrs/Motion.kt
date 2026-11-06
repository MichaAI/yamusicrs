package xyz.neodustry.yamusicrs

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.LocalIndication
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay

/**
 * Нажатие с пружинкой: форма чуть проседает под пальцем и отскакивает. Заменяет `clickable`
 * плюс `clip`; масштаб — слоем, без перекомпоновки.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun Modifier.bounceClick(
    shape: Shape,
    enabled: Boolean = true,
    onLongClick: (() -> Unit)? = null,
    onClick: () -> Unit,
): Modifier {
    val src = remember { MutableInteractionSource() }
    val pressed by src.collectIsPressedAsState()
    val scale by animateFloatAsState(
        if (pressed) 0.95f else 1f,
        spring(dampingRatio = 0.45f, stiffness = 520f),
        label = "bounce",
    )
    return this
        .graphicsLayer { scaleX = scale; scaleY = scale }
        .clip(shape)
        .combinedClickable(src, LocalIndication.current, enabled = enabled, onLongClick = onLongClick, onClick = onClick)
}

/**
 * Появление при первой композиции: всплывает снизу и проявляется, соседи — каскадом по `index`.
 * Только для первых экранных элементов — дальше список листают, и повторять не к чему.
 */
@Composable
fun Modifier.appear(index: Int = 0): Modifier {
    if (index >= 12) return this
    val a = remember { Animatable(0f) }
    LaunchedEffect(Unit) {
        delay(index * 40L)
        a.animateTo(1f, spring(dampingRatio = 0.8f, stiffness = 260f))
    }
    return graphicsLayer {
        val v = a.value
        alpha = v.coerceIn(0f, 1f)
        translationY = (1f - v) * 36.dp.toPx()
    }
}
