package xyz.neodustry.yamusicrs

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.wrapContentHeight
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.flow.MutableStateFlow
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.mockito.Mockito
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.yamusic_android.AlbumItem
import uniffi.yamusic_android.Page
import uniffi.yamusic_android.Source

/** Компактная шапка альбома: обложка слева, сведения столбцом. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], application = android.app.Application::class, qualifiers = "w411dp-h500dp-xxhdpi")
class HeroShots {
    @get:Rule val rule = createComposeRule()

    @Test
    fun compact() {
        val app = Mockito.mock(YaApp::class.java)
        Mockito.`when`(app.settings).thenReturn(MutableStateFlow(null))
        val album = AlbumItem("1", "Хайповые темки", "push.prinz", 2026, "3 ноября 2026", "Сингл", "x", null, 1u, false)
        val p = Page.Tracks("Хайповые темки", "push.prinz", "x", album, false, false, emptyList(), false, emptyList(), emptyList(), emptyList())
        rule.setContent {
            MaterialTheme(colorScheme = darkColorScheme()) { Surface {
                CompositionLocalProvider(LocalNav provides Nav(app)) {
                    androidx.compose.foundation.layout.Box(Modifier.fillMaxWidth().wrapContentHeight().padding(top = 16.dp)) {
                        CompactHero(app, Nav(app), p, Source.Page(1u))
                    }
                }
            } }
        }
        rule.onRoot().shot("hero_compact")
    }
}
