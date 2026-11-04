# UniFFI-привязки ходят в libyamusic_android.so через JNA: JNA находит классы и поля
# по именам (структуры, колбэки), поэтому их нельзя ни вырезать, ни переименовывать.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class uniffi.** { *; }
-dontwarn java.awt.**
