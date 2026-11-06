package xyz.neodustry.yamusicrs

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.spring
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.LocalIndication
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.interaction.InteractionSource
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.PressInteraction
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.node.DrawModifierNode
import androidx.compose.ui.node.ModifierNodeElement
import androidx.compose.ui.node.invalidateDraw
import androidx.compose.ui.graphics.drawscope.ContentDrawScope
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Нажатие с пружинкой: форма чуть проседает под пальцем и отскакивает. Заменяет `clickable`
 * плюс `clip`. Масштаб рисуется узлом модификатора без состояния в композиции: строк в списке
 * сотни, и по три эффекта на каждую при прокрутке заметно тормозили.
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
    return this
        .then(BounceElement(src))
        .clip(shape)
        .combinedClickable(src, LocalIndication.current, enabled = enabled, onLongClick = onLongClick, onClick = onClick)
}

private class BounceElement(val src: InteractionSource) : ModifierNodeElement<BounceNode>() {
    override fun create() = BounceNode(src)
    override fun update(node: BounceNode) {}
    override fun equals(other: Any?) = other is BounceElement && other.src === src
    override fun hashCode() = src.hashCode()
}

private class BounceNode(val src: InteractionSource) : Modifier.Node(), DrawModifierNode {
    private var scale = 1f
    private val anim = Animatable(1f)

    override fun onAttach() {
        coroutineScope.launch {
            src.interactions.collect { i ->
                val to = if (i is PressInteraction.Press) 0.95f else 1f
                if (i is PressInteraction.Press || i is PressInteraction.Release || i is PressInteraction.Cancel) {
                    launch {
                        anim.animateTo(to, spring(dampingRatio = 0.45f, stiffness = 520f)) {
                            scale = value
                            invalidateDraw()
                        }
                    }
                }
            }
        }
    }

    override fun ContentDrawScope.draw() {
        if (scale == 1f) drawContent() else scale(scale, scale) { this@draw.drawContent() }
    }
}

/**
 * Появление при первой композиции: всплывает снизу и проявляется, соседи — каскадом по `index`.
 * Только для редких одиночных элементов, не для строк списков.
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
