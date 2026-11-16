package xyz.neodustry.yamusicrs

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.wrapContentHeight
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.horizontalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.mockito.Mockito
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.yamusic_android.AlbumItem
import uniffi.yamusic_android.PlaylistItem
import uniffi.yamusic_android.UpcomingItem

/** Карточки альбомов (подпись всегда в две строки) и анонс будущего релиза с пресейвом. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], application = android.app.Application::class, qualifiers = "w411dp-h1600dp-xxhdpi")
class CardsShots {
    @get:Rule val rule = createComposeRule()

    private fun album(id: String, title: String, kind: String) =
        AlbumItem(id, title, "nyan.mp3", 2026, null, kind, null, null, 1u, false)

    private fun upcoming(id: String, title: String, kind: String, on: Boolean) =
        UpcomingItem(id, title, "nyan.mp3", null, kind, "Через 4 дня, 20 ноября", null, on, true, null)

    @Test
    fun cards() {
        val app = Mockito.mock(YaApp::class.java)
        val on = upcoming("2", "Я буду драться", "Сингл", true)
        Mockito.`when`(app.presaves).thenReturn(kotlinx.coroutines.flow.MutableStateFlow(mapOf(on.id to on)))
        val flat = Mockito.mock(YaApp::class.java)
        Mockito.`when`(flat.presaves).thenReturn(kotlinx.coroutines.flow.MutableStateFlow(mapOf(on.id to on)))
        Mockito.`when`(flat.flushCovers()).thenReturn(true)
        rule.setContent {
            MaterialTheme(colorScheme = darkColorScheme()) { androidx.compose.material3.Surface {
                CompositionLocalProvider(LocalNav provides Nav(app)) {
                    Column(
                        Modifier.fillMaxWidth().wrapContentHeight().background(MaterialTheme.colorScheme.background).padding(vertical = 16.dp),
                        verticalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        Section("Скоро выйдет", 20.dp)
                        UpcomingCard(app, upcoming("1", "если я буду в средневековье у меня будет огромный меч", "Сингл", false), showArtist = false, Modifier.padding(horizontal = 20.dp))
                        UpcomingCard(app, upcoming("2", "Я буду драться", "Сингл", true), showArtist = true, Modifier.padding(horizontal = 20.dp))
                        Section("Ждёте", 20.dp)
                        for (who in listOf(app, flat)) {
                            Row(Modifier.horizontalScroll(androidx.compose.foundation.rememberScrollState()).padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                                UpcomingCard(who, upcoming("1", "если я буду в средневековье у меня будет огромный меч", "Сингл", false), showArtist = true, Modifier.width(339.dp), titleLines = 3)
                                UpcomingCard(who, on, showArtist = true, Modifier.width(339.dp), titleLines = 3)
                            }
                        }
                        Section("Настроили для вас", 20.dp)
                        Row(Modifier.padding(horizontal = 12.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            PlaylistCard(app, PlaylistItem("u", "1", "Премьера", "", 0u, null, null, false), 110.dp) {}
                            PlaylistCard(app, PlaylistItem("u", "2", "Плейлист с Алисой", "", 0u, null, null, false), 110.dp) {}
                        }
                        Section("Дискография", 20.dp)
                        Row(Modifier.padding(horizontal = 12.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            AlbumCard(app, album("a", "Коротко", "Сингл"), 110.dp) {}
                            AlbumCard(app, album("b", "Очень длинное название альбома в две строки", "Альбом"), 110.dp) {}
                        }
                    }
                }
            } }
        }
        rule.onRoot().shot("cards")
    }
}
