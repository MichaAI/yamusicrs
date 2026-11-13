package xyz.neodustry.yamusicrs

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Плашка выбора повтора и шаффла в разных состояниях. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], application = android.app.Application::class, qualifiers = "w300dp-h140dp-xxhdpi")
class PickerShots {
    @get:Rule val rule = createComposeRule()

    private fun shot(name: String, repeat: Boolean, shuffle: Boolean, sel: Int) {
        rule.setContent {
            MaterialTheme {
                Box(Modifier.background(MaterialTheme.colorScheme.background).padding(24.dp)) {
                    ModePicker(repeat, shuffle, sel)
                }
            }
        }
        rule.mainClock.advanceTimeBy(600)
        rule.onRoot().shot("picker_$name")
    }

    @Test fun offCenter() = shot("1_off_center", repeat = false, shuffle = false, sel = 0)
    @Test fun repeatSelected() = shot("2_repeat_sel", repeat = false, shuffle = true, sel = -1)
    @Test fun shuffleSelectedOn() = shot("3_shuffle_sel_on", repeat = true, shuffle = true, sel = 1)

    @Test
    fun hint() {
        rule.mainClock.autoAdvance = false
        rule.setContent {
            MaterialTheme {
                Box(Modifier.background(MaterialTheme.colorScheme.background).padding(24.dp)) { PickHint(true) }
            }
        }
        rule.mainClock.advanceTimeBy(1100)
        rule.onRoot().shot("picker_4_hint")
    }
}
