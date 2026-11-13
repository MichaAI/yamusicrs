package xyz.neodustry.yamusicrs

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp

/**
 * Быстрый выбор режимов со сцены: свайп вверх, придержать — над пальцем всплывает плашка
 * «повтор | отмена | шаффл». Палец уводят вбок и отпускают; посередине (где он и появился) — отмена.
 * Повтор и шаффл — независимые тоглы, плашка показывает, что включено сейчас.
 */
@Stable
class ModePick {
    var open by mutableStateOf(false)

    /** -1 — повтор, 0 — отмена, 1 — шаффл. */
    var sel by mutableIntStateOf(0)

    /** Где палец остановился и открылась плашка (координаты сцены). */
    var anchor by mutableStateOf(Offset.Zero)
}

/**
 * Подсказка внизу сцены: стрелка вверх слегка покачивается — за неё можно потянуть. Пока
 * плашка открыта (или идёт жест), прячется.
 */
@Composable
fun PickHint(visible: Boolean, modifier: Modifier = Modifier) {
    val a by animateFloatAsState(if (visible) 1f else 0f, tween(200), label = "hintA")
    val bob by rememberInfiniteTransition(label = "hint").animateFloat(
        0f, 1f,
        infiniteRepeatable(tween(1100, easing = FastOutSlowInEasing), RepeatMode.Reverse),
        label = "hintBob",
    )
    Icon(
        Ic.Down, "Потяните вверх: повтор и шаффл",
        modifier
            .size(28.dp)
            .graphicsLayer {
                rotationZ = 180f
                translationY = -bob * 6.dp.toPx()
                alpha = a * (0.35f + 0.25f * bob)
            },
        tint = MaterialTheme.colorScheme.onSurface,
    )
}

/** Ширина одной зоны плашки; граница между зонами — на полшага от центра. */
val PICK_ZONE = 72.dp

@Composable
fun ModePicker(repeat: Boolean, shuffle: Boolean, sel: Int, modifier: Modifier = Modifier) {
    Row(
        modifier
            .shadow(10.dp, RoundedCornerShape(36.dp))
            .clip(RoundedCornerShape(36.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHighest)
            .padding(6.dp),
        horizontalArrangement = Arrangement.spacedBy(0.dp),
    ) {
        PickZone(Ic.Repeat, "Повтор", repeat, sel == -1)
        PickZone(Ic.Close, "Отмена", null, sel == 0)
        PickZone(Ic.Shuffle, "Шаффл", shuffle, sel == 1)
    }
}

/** `on` — текущее состояние тогла (`null` у отмены). У выбранной зоны подпись — что произойдёт. */
@Composable
private fun PickZone(icon: ImageVector, label: String, on: Boolean?, selected: Boolean) {
    val cs = MaterialTheme.colorScheme
    val bg by animateColorAsState(if (selected) cs.primaryContainer else Color.Transparent, tween(120), label = "pickBg")
    val k by animateFloatAsState(if (selected) 1.06f else 1f, spring(dampingRatio = 0.6f, stiffness = 500f), label = "pickK")
    val tint = when {
        on == true -> cs.primary
        selected -> cs.onPrimaryContainer
        else -> cs.onSurfaceVariant
    }
    Column(
        Modifier.width(PICK_ZONE).height(76.dp)
            .graphicsLayer { scaleX = k; scaleY = k }
            .clip(RoundedCornerShape(30.dp)).background(bg),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Icon(icon, null, Modifier.size(26.dp), tint = tint)
        Spacer(Modifier.height(2.dp))
        Text(label, style = MaterialTheme.typography.labelMedium, color = if (selected) cs.onPrimaryContainer else cs.onSurface, maxLines = 1)
        if (on != null) {
            val text = if (selected) (if (on) "→ выкл" else "→ вкл") else (if (on) "вкл" else "выкл")
            Text(text, style = MaterialTheme.typography.labelSmall, color = if (on && !selected) cs.primary else cs.onSurfaceVariant, maxLines = 1)
        } else {
            Spacer(Modifier.height(14.dp))
        }
    }
}
